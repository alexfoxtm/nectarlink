// SPDX-License-Identifier: GPL-3.0-or-later
// Original, opt-in public HF-unit prototype; no product capability admission.
// Main-thread-only C entry points are documented in this packet's FFI-CONTRACT.md.
// No PCM capture, default-device writes, pairing, permission requests or hangup.
#import <Foundation/Foundation.h>
#import <dispatch/dispatch.h>
#import <IOBluetooth/IOBluetooth.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreBluetooth/CoreBluetooth.h>
#import <AVFoundation/AVFoundation.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// These callbacks are synchronous, main-thread-only and must not reenter this API.
// The lease callback must validate the entire exact binding against current state.
typedef int (*WPHfpLeaseIsCurrent)(void *context, const char *bindingJSON);
typedef void (*WPHfpEvent)(void *context, const char *snapshotJSON);

static BOOL WPString(id value) {
    if (![value isKindOfClass:NSString.class]) return NO;
    NSString *s = value;
    return s.length > 0 && s.length <= 512 &&
        [s rangeOfCharacterFromSet:NSCharacterSet.controlCharacterSet].location == NSNotFound;
}

static NSString *WPAddress(id raw) {
    if (!WPString(raw)) return nil;
    NSString *s = [[raw stringByReplacingOccurrencesOfString:@":" withString:@"-"] uppercaseString];
    NSArray *parts = [s componentsSeparatedByString:@"-"];
    if (parts.count != 6) return nil;
    NSCharacterSet *nonHex = [[NSCharacterSet characterSetWithCharactersInString:@"0123456789ABCDEF"] invertedSet];
    for (NSString *part in parts) {
        if (part.length != 2 || [part rangeOfCharacterFromSet:nonHex].location != NSNotFound) return nil;
    }
    return s;
}

static BOOL WPBindingValid(NSDictionary *b) {
    if (![b isKindOfClass:NSDictionary.class]) return NO;
    for (NSString *key in @[@"phoneId", @"trustedKeySHA256", @"callId", @"generation", @"remoteNumber", @"inputUID", @"outputUID"]) {
        if (!WPString(b[key])) return NO;
    }
    NSString *digest = b[@"trustedKeySHA256"];
    if (digest.length != 64 || [digest rangeOfCharacterFromSet:
        [[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdef"] invertedSet]].location != NSNotFound) return NO;
    if (!WPAddress(b[@"bluetoothAddress"])) return NO;
    if (![b[@"incoming"] isKindOfClass:NSNumber.class] ||
        !([b[@"incoming"] isEqual:@YES] || [b[@"incoming"] isEqual:@NO])) return NO;
    return YES;
}

// A list callback alone cannot prove exclusivity; Root's live lease must also
// assert exactly one active voice call, with no setup/held/multiparty calls.
static BOOL WPCallMatches(NSDictionary *b, NSDictionary *call) {
    if (!WPBindingValid(b) || ![call isKindOfClass:NSDictionary.class]) return NO;
    for (NSString *key in @[IOBluetoothHandsFreeCallIndex, IOBluetoothHandsFreeCallDirection,
                            IOBluetoothHandsFreeCallStatus, IOBluetoothHandsFreeCallMode,
                            IOBluetoothHandsFreeCallMultiparty]) {
        if (![call[key] isKindOfClass:NSNumber.class]) return NO;
    }
    return [call[IOBluetoothHandsFreeCallIndex] integerValue] > 0 &&
        [call[IOBluetoothHandsFreeCallDirection] isEqual:b[@"incoming"]] &&
        [call[IOBluetoothHandsFreeCallStatus] isEqual:@0] &&
        [call[IOBluetoothHandsFreeCallMode] isEqual:@0] &&
        [call[IOBluetoothHandsFreeCallMultiparty] isEqual:@0] &&
        [call[IOBluetoothHandsFreeCallNumber] isKindOfClass:NSString.class] &&
        [call[IOBluetoothHandsFreeCallNumber] isEqual:b[@"remoteNumber"]];
}

static BOOL WPScalar(AudioObjectID device, AudioObjectPropertySelector selector, UInt32 *out) {
    AudioObjectPropertyAddress a = {selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 size = sizeof(*out);
    return AudioObjectGetPropertyData(device, &a, 0, NULL, &size, out) == noErr && size == sizeof(*out);
}

static NSString *WPUID(AudioDeviceID device) {
    AudioObjectPropertyAddress a = {kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal,
                                    kAudioObjectPropertyElementMain};
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    OSStatus result = AudioObjectGetPropertyData(device, &a, 0, NULL, &size, &value);
    if (result != noErr || size != sizeof(value) || !value) {
        if (value) CFRelease(value);
        return nil;
    }
    return CFBridgingRelease(value);
}

static NSArray<NSNumber *> *WPStreams(AudioDeviceID device, AudioObjectPropertyScope scope) {
    AudioObjectPropertyAddress a = {kAudioDevicePropertyStreams, scope, kAudioObjectPropertyElementMain};
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(device, &a, 0, NULL, &size) != noErr ||
        size == 0 || size % sizeof(AudioStreamID) != 0 || size > 64 * sizeof(AudioStreamID)) return nil;
    AudioStreamID streams[64] = {0};
    UInt32 capacity = size;
    if (AudioObjectGetPropertyData(device, &a, 0, NULL, &size, streams) != noErr ||
        size > capacity || size % sizeof(AudioStreamID) != 0) return nil;
    NSMutableArray *result = [NSMutableArray array];
    for (UInt32 i = 0; i < size / sizeof(AudioStreamID); ++i) [result addObject:@(streams[i])];
    return result.count ? result : nil;
}

static BOOL WPLocalTransport(UInt32 transport) {
    return transport == kAudioDeviceTransportTypeBuiltIn || transport == kAudioDeviceTransportTypeUSB ||
        transport == kAudioDeviceTransportTypePCI || transport == kAudioDeviceTransportTypeThunderbolt ||
        transport == kAudioDeviceTransportTypeDisplayPort || transport == kAudioDeviceTransportTypeHDMI ||
        transport == kAudioDeviceTransportTypeFireWire;
}

// Read only. UIDs are opaque; no substring or friendly-name Bluetooth mapping.
static NSDictionary *WPEndpoint(AudioDeviceID device, AudioObjectPropertyScope scope, NSString *expectedUID) {
    UInt32 alive = 0, running = 0, transport = 0;
    NSArray *streams = WPStreams(device, scope);
    NSString *uid = WPUID(device);
    if (!uid || ![uid isEqual:expectedUID] || !streams ||
        !WPScalar(device, kAudioDevicePropertyDeviceIsAlive, &alive) || alive != 1 ||
        !WPScalar(device, kAudioDevicePropertyDeviceIsRunning, &running) ||
        !WPScalar(device, kAudioDevicePropertyTransportType, &transport) || !WPLocalTransport(transport)) return nil;
    return @{@"deviceId": @(device), @"uid": uid, @"alive": @YES,
             @"running": @(running == 1), @"transport": @(transport), @"streams": streams};
}

static NSDictionary *WPAudioObservation(NSDictionary *binding) {
    UInt32 input = 0, output = 0;
    if (!WPScalar(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultInputDevice, &input) ||
        !WPScalar(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice, &output) || !input || !output) return nil;
    NSDictionary *mic = WPEndpoint(input, kAudioObjectPropertyScopeInput, binding[@"inputUID"]);
    NSDictionary *speaker = WPEndpoint(output, kAudioObjectPropertyScopeOutput, binding[@"outputUID"]);
    if (!mic || !speaker) return nil;
    // This observes the chosen current defaults; it does not infer that the
    // Bluetooth stack consumes them or proves either direction of call audio.
    return @{@"input": mic, @"output": speaker, @"duplexQualified": @NO};
}

static BOOL WPPermissionsGranted(void) {
    // Status queries only; neither API allocates a manager or requests access.
    return CBManager.authorization == CBManagerAuthorizationAllowedAlways &&
        [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio] == AVAuthorizationStatusAuthorized;
}

typedef struct {
    BOOL serviceConnectPending;
    BOOL scoTransferPending;
    BOOL serviceClosed;
    BOOL scoNeedsClosure;
    BOOL scoClosed;
} WPHfpRetirement;

// Production retirement policy, also exercised without native objects below.
static BOOL WPRetirementReady(WPHfpRetirement state, BOOL stopRequested,
                              BOOL serviceConnected, BOOL scoConnected) {
    return stopRequested && !state.serviceConnectPending && !state.scoTransferPending &&
        state.serviceClosed && (!state.scoNeedsClosure || state.scoClosed) &&
        !serviceConnected && !scoConnected;
}

@interface WPHfpProvider : NSObject <IOBluetoothHandsFreeDeviceDelegate>
@property(nonatomic, readonly) BOOL retired;
@property(nonatomic, readonly) BOOL inCallback;
- (instancetype)initWithBinding:(NSDictionary *)binding json:(NSString *)json
                         lease:(WPHfpLeaseIsCurrent)lease event:(WPHfpEvent)event context:(void *)context;
- (void)start;
- (void)stop:(NSString *)reason;
- (NSDictionary *)snapshot;
- (void)detach;
- (BOOL)authorized;
- (void)emit;
- (void)failBeforeConnect:(NSString *)reason;
- (void)closeOwned;
- (void)observeRetirement;
- (void)tick;
- (void)onMain:(dispatch_block_t)work;
- (BOOL)isOurDevice:(IOBluetoothHandsFree *)device;
@end

@implementation WPHfpProvider {
    NSDictionary *_binding;
    NSString *_bindingJSON;
    WPHfpLeaseIsCurrent _lease;
    WPHfpEvent _event;
    void *_context;
    IOBluetoothHandsFreeDevice *_hf;
    NSTimer *_timer;
    NSString *_phase;
    NSString *_reason;
    NSDictionary *_audio;
    NSNumber *_callIndex;
    NSTimeInterval _deadline;
    NSTimeInterval _lastCallQuery;
    BOOL _stopRequested;
    WPHfpRetirement _retirement;
    BOOL _ownsConnection;
    BOOL _retired;
    BOOL _inCallback;
    BOOL _detached;
    NSInteger _nativeStatus;
}

- (instancetype)initWithBinding:(NSDictionary *)binding json:(NSString *)json
                         lease:(WPHfpLeaseIsCurrent)lease event:(WPHfpEvent)event context:(void *)context {
    self = [super init];
    if (self) {
        _binding = [binding copy];
        _bindingJSON = [json copy];
        _lease = lease;
        _event = event;
        _context = context;
        _phase = @"created";
        _reason = @"operator-prototype-only";
    }
    return self;
}

- (BOOL)retired { return _retired; }
- (BOOL)inCallback { return _inCallback; }

- (BOOL)authorized {
    if (_stopRequested || _detached || !_lease || _inCallback) return NO;
    _inCallback = YES;
    BOOL current = _lease(_context, _bindingJSON.UTF8String) == 1;
    _inCallback = NO;
    return current && WPPermissionsGranted();
}

- (NSDictionary *)snapshot {
    return @{@"phase": _phase, @"reason": _reason, @"nativeStatus": @(_nativeStatus),
             @"slcObserved": @(_hf && _hf.isConnected), @"scoObserved": @(_hf && _hf.isSCOConnected),
             @"stopLatched": @(_stopRequested), @"retired": @(_retired),
             @"available": @NO, @"connected": @NO, @"duplexQualified": @NO,
             @"handsetAudibleVerified": @NO, @"audioObservation": _audio ?: NSNull.null};
}

- (void)emit {
    if (!_event || _detached || _inCallback) return;
    NSData *data = [NSJSONSerialization dataWithJSONObject:self.snapshot options:NSJSONWritingSortedKeys error:NULL];
    NSString *json = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : @"{}";
    _inCallback = YES;
    _event(_context, json.UTF8String);
    _inCallback = NO;
}

- (void)failBeforeConnect:(NSString *)reason {
    _phase = @"rejected";
    _reason = reason;
    _stopRequested = YES;
    _retired = YES;
    _hf.delegate = nil;
    _hf = nil;
    [self emit];
}

- (void)start {
    if (![NSThread isMainThread] || ![_phase isEqual:@"created"]) return;
    if (![self authorized]) { [self failBeforeConnect:@"permission-or-current-lease-required"]; return; }
    _audio = WPAudioObservation(_binding);
    if (!_audio) { [self failBeforeConnect:@"selected-current-mic-speaker-not-observed"]; return; }
    IOBluetoothDevice *phone = [IOBluetoothDevice deviceWithAddressString:WPAddress(_binding[@"bluetoothAddress"])];
    if (!phone || !phone.isPaired || ![WPAddress(phone.addressString) isEqual:WPAddress(_binding[@"bluetoothAddress"])]) {
        [self failBeforeConnect:@"exact-already-paired-phone-required"]; return;
    }
    if (!phone.isHandsFreeAudioGateway) {
        [self failBeforeConnect:@"phone-audio-gateway-service-not-in-cached-sdp"]; return;
    }
    _hf = [[IOBluetoothHandsFreeDevice alloc] initWithDevice:phone delegate:self];
    if (!_hf || _hf.isConnected || _hf.isSCOConnected) {
        [self failBeforeConnect:@"new-exclusive-hf-owner-required"]; return;
    }
    _phase = @"connecting-service";
    _deadline = NSDate.timeIntervalSinceReferenceDate + 12.0;
    _retirement.serviceConnectPending = YES;
    _ownsConnection = YES;
    __weak WPHfpProvider *weakSelf = self;
    _timer = [NSTimer timerWithTimeInterval:0.2 repeats:YES block:^(NSTimer *timer) {
        WPHfpProvider *owner = weakSelf;
        if (owner) [owner tick]; else [timer invalidate];
    }];
    [NSRunLoop.mainRunLoop addTimer:_timer forMode:NSRunLoopCommonModes];
    [_hf connect];
    [self emit];
}

- (void)closeOwned {
    if (!_ownsConnection || !_hf) return;
    // Never endCall, closeConnection, clear pairing, or reset a system route.
    if (_hf.isConnected) [_hf transferAudioToPhone];
    if (_hf.isSCOConnected) [_hf disconnectSCO];
    [_hf disconnect];
}

- (void)stop:(NSString *)reason {
    if (_retired || _detached) return;
    if (!_stopRequested) {
        _stopRequested = YES; // Permanent: no callback can rearm this owner.
        _phase = @"returning-to-handset";
        _reason = [reason copy];
        _deadline = NSDate.timeIntervalSinceReferenceDate + 12.0;
        [self closeOwned];
        [self emit];
    }
    [self observeRetirement];
}

- (void)observeRetirement {
    if (_retired || !WPRetirementReady(_retirement, _stopRequested,
                                      _hf.isConnected, _hf.isSCOConnected)) return;
    _hf.delegate = nil;
    _hf = nil;
    _ownsConnection = NO;
    _retired = YES;
    _phase = @"service-and-sco-disconnected";
    [_timer invalidate];
    _timer = nil;
    [self emit];
}

- (void)tick {
    if (_retired || _detached) return;
    NSTimeInterval now = NSDate.timeIntervalSinceReferenceDate;
    if (_stopRequested) {
        [self observeRetirement];
        if (!_retired && now >= _deadline && ![_phase isEqual:@"handback-unconfirmed"]) {
            _phase = @"handback-unconfirmed";
            // Keep the owner, timer and delegate alive for late connect/close.
            [self closeOwned];
            [self emit];
        }
        return;
    }
    if (![self authorized]) { [self stop:@"lease-or-permission-revoked"]; return; }
    NSDictionary *audio = WPAudioObservation(_binding);
    if (!audio) { [self stop:@"selected-current-audio-endpoints-changed"]; return; }
    _audio = audio;
    if (now >= _deadline && ![_phase isEqual:@"sco-observed-unqualified"]) {
        [self stop:@"native-phase-timeout"]; return;
    }
    if ([_phase isEqual:@"sco-observed-unqualified"] && (!_hf.isConnected || !_hf.isSCOConnected)) {
        [self stop:@"service-or-sco-no-longer-connected"]; return;
    }
    if (_hf.isConnected && now - _lastCallQuery >= 1.0) {
        _lastCallQuery = now;
        [_hf currentCallList];
    }
}

// Delegate delivery is serialized onto main; each closure retains this exact
// owner, never a global current owner. A retired/stopped generation stays inert.
- (void)onMain:(dispatch_block_t)work {
    if (NSThread.isMainThread) work(); else dispatch_async(dispatch_get_main_queue(), work);
}

- (BOOL)isOurDevice:(IOBluetoothHandsFree *)device {
    return !_detached && !_retired && device == _hf;
}

- (void)handsFree:(IOBluetoothHandsFree *)device connected:(NSNumber *)status {
    [self onMain:^{
        if (![self isOurDevice:device]) return;
        self->_retirement.serviceConnectPending = NO;
        self->_nativeStatus = status.integerValue;
        if (status.integerValue != 0 || !device.isConnected) {
            // Service failure settles only the service operation; independent
            // SCO evidence and both current properties still fence retirement.
            self->_retirement.serviceClosed = !device.isConnected;
            [self stop:@"service-connect-failed"];
            return;
        }
        self->_retirement.serviceClosed = NO;
        if (self->_stopRequested) { [self closeOwned]; return; }
        if (![self authorized]) { [self stop:@"lease-revoked-before-call-check"]; return; }
        self->_phase = @"checking-exact-call";
        self->_deadline = NSDate.timeIntervalSinceReferenceDate + 8.0;
        self->_lastCallQuery = NSDate.timeIntervalSinceReferenceDate;
        [self->_hf currentCallList];
        [self emit];
    }];
}

- (void)handsFree:(IOBluetoothHandsFreeDevice *)device currentCall:(NSDictionary *)call {
    NSDictionary *record = [call copy];
    [self onMain:^{
        if (![self isOurDevice:device] || self->_stopRequested) return;
        if (![self authorized] || !WPCallMatches(self->_binding, record) ||
            (self->_callIndex && ![self->_callIndex isEqual:record[IOBluetoothHandsFreeCallIndex]])) {
            [self stop:@"hfp-call-does-not-match-current-bound-call"]; return;
        }
        self->_callIndex = record[IOBluetoothHandsFreeCallIndex];
        if ([self->_phase isEqual:@"checking-exact-call"]) {
            self->_audio = WPAudioObservation(self->_binding);
            if (!self->_audio || ![self authorized]) { [self stop:@"route-or-lease-changed-before-transfer"]; return; }
            self->_phase = @"connecting-audio";
            self->_deadline = NSDate.timeIntervalSinceReferenceDate + 8.0;
            self->_retirement.scoTransferPending = YES;
            self->_retirement.scoNeedsClosure = YES;
            self->_retirement.scoClosed = NO;
            [self->_hf transferAudioToComputer];
            [self emit];
        }
    }];
}

- (void)handsFree:(IOBluetoothHandsFree *)device scoConnectionOpened:(NSNumber *)status {
    [self onMain:^{
        if (![self isOurDevice:device]) return;
        self->_nativeStatus = status.integerValue;
        BOOL transferWasPending = self->_retirement.scoTransferPending;
        self->_retirement.scoTransferPending = NO;
        if (device.isSCOConnected) {
            self->_retirement.scoNeedsClosure = YES;
            self->_retirement.scoClosed = NO;
        } else if (status.integerValue != 0 && transferWasPending) {
            // The owned transfer failed with no SCO connection. This terminal
            // result settles it without inventing a successful close callback.
            self->_retirement.scoClosed = YES;
        } else if (status.integerValue == 0) {
            // A successful close callback may already have been delivered;
            // a delayed open result with no live SCO must not erase it.
            self->_retirement.scoNeedsClosure = YES;
        }
        if (self->_stopRequested) {
            [self closeOwned];
            [self observeRetirement];
            return;
        }
        if (status.integerValue != 0 || !device.isSCOConnected || !self->_callIndex ||
            ![self->_phase isEqual:@"connecting-audio"] || ![self authorized]) {
            [self stop:@"sco-failed-or-arrived-without-bound-transfer"]; return;
        }
        self->_audio = WPAudioObservation(self->_binding);
        if (!self->_audio) { [self stop:@"audio-endpoints-changed-at-sco-open"]; return; }
        self->_phase = @"sco-observed-unqualified";
        self->_reason = @"physical-duplex-and-handset-return-not-qualified";
        [self emit];
    }];
}

- (void)handsFree:(IOBluetoothHandsFree *)device disconnected:(NSNumber *)status {
    [self onMain:^{
        if (![self isOurDevice:device]) return;
        self->_nativeStatus = status.integerValue;
        // Service closure is independent of SCO callback order and cannot
        // settle either pending service connect or pending audio transfer.
        if (device.isConnected) self->_retirement.serviceClosed = NO;
        else if (status.integerValue == 0) self->_retirement.serviceClosed = YES;
        [self stop:@"service-disconnected"];
    }];
}

- (void)handsFree:(IOBluetoothHandsFree *)device scoConnectionClosed:(NSNumber *)status {
    [self onMain:^{
        if (![self isOurDevice:device]) return;
        self->_nativeStatus = status.integerValue;
        if (device.isSCOConnected) self->_retirement.scoClosed = NO;
        else if (status.integerValue == 0) self->_retirement.scoClosed = YES;
        // A close may precede a pending open result; only that result settles
        // scoTransferPending, so a late open still belongs to this Stop owner.
        [self stop:@"sco-closed"];
    }];
}

- (void)handsFree:(IOBluetoothHandsFreeDevice *)device isCallActive:(NSNumber *)active {
    [self onMain:^{ if ([self isOurDevice:device] && ![active isEqual:@1]) [self stop:@"phone-call-no-longer-active"]; }];
}
- (void)handsFree:(IOBluetoothHandsFreeDevice *)device callSetupMode:(NSNumber *)setup {
    [self onMain:^{ if ([self isOurDevice:device] && ![setup isEqual:@0]) [self stop:@"phone-call-setup-changed"]; }];
}
- (void)handsFree:(IOBluetoothHandsFreeDevice *)device callHoldState:(NSNumber *)held {
    [self onMain:^{ if ([self isOurDevice:device] && ![held isEqual:@0]) [self stop:@"phone-held-call-present"]; }];
}

- (void)detach {
    if (!_retired) return;
    _detached = YES;
    [_timer invalidate];
    _timer = nil;
    _event = NULL;
    _lease = NULL;
    _context = NULL;
}
@end

// Exactly one native experiment may own HF at a time, including while handback
// is unconfirmed. The process-local monotonic handle prevents stale-owner ABA.
static WPHfpProvider *WPActive;
static uint64_t WPToken; // Never reuse a retired handle, including allocator address reuse.

uint64_t WPHfpStart(const char *bindingJSON, WPHfpLeaseIsCurrent lease, WPHfpEvent event, void *context) {
    if (!NSThread.isMainThread || WPActive || WPToken == UINT64_MAX || !bindingJSON || !lease || !event) return 0;
    size_t length = strnlen(bindingJSON, 8193);
    if (length == 0 || length > 8192) return 0;
    NSData *data = [NSData dataWithBytes:bindingJSON length:length];
    NSDictionary *binding = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    if (!WPBindingValid(binding)) return 0;
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    WPActive = [[WPHfpProvider alloc] initWithBinding:binding json:json lease:lease event:event context:context];
    ++WPToken;
    [WPActive start];
    return WPToken;
}

int WPHfpHandback(uint64_t handle) {
    if (!NSThread.isMainThread || !WPActive || handle != WPToken || WPActive.inCallback) return 0;
    [WPActive stop:@"operator-or-root-stop"];
    return 1;
}

char *WPHfpCopySnapshotJSON(uint64_t handle) {
    if (!NSThread.isMainThread || !WPActive || handle != WPToken || WPActive.inCallback) return NULL;
    NSData *data = [NSJSONSerialization dataWithJSONObject:WPActive.snapshot options:NSJSONWritingSortedKeys error:NULL];
    NSString *json = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
    return json ? strdup(json.UTF8String) : NULL;
}

void WPHfpFreeJSON(char *json) { free(json); }

int WPHfpRelease(uint64_t handle) {
    if (!NSThread.isMainThread || !WPActive || handle != WPToken ||
        WPActive.inCallback || !WPActive.retired) return 0;
    [WPActive detach];
    WPActive = nil;
    return 1;
}

#if defined(WAYPAIR_HFP_POLICY_TEST_MAIN)
// One main/failure pair per call policy and production retirement policy.
// Pure Foundation data and retirement-state values only;
// no provider instance, permission check, Bluetooth, HAL, device or audio call.
static NSDictionary *WPTestBinding(void) {
    return @{@"phoneId": @"test-phone", @"trustedKeySHA256": [@"a" stringByPaddingToLength:64 withString:@"a" startingAtIndex:0],
             @"callId": @"test-call", @"generation": @"7", @"remoteNumber": @"+15550000000", @"incoming": @YES,
             @"bluetoothAddress": @"00-11-22-33-44-55", @"inputUID": @"test-mic", @"outputUID": @"test-speaker"};
}
static NSDictionary *WPTestCall(void) {
    return @{IOBluetoothHandsFreeCallIndex: @1, IOBluetoothHandsFreeCallDirection: @1,
             IOBluetoothHandsFreeCallStatus: @0, IOBluetoothHandsFreeCallMode: @0,
             IOBluetoothHandsFreeCallMultiparty: @0, IOBluetoothHandsFreeCallNumber: @"+15550000000"};
}
static BOOL WPMainPathTest(void) { return WPCallMatches(WPTestBinding(), WPTestCall()); }
static BOOL WPFailurePathTest(void) {
    NSMutableDictionary *wrong = [WPTestCall() mutableCopy];
    wrong[IOBluetoothHandsFreeCallNumber] = @"+15550000001";
    if (WPCallMatches(WPTestBinding(), wrong)) return NO;
    wrong = [WPTestCall() mutableCopy];
    [wrong removeObjectForKey:IOBluetoothHandsFreeCallNumber];
    if (WPCallMatches(WPTestBinding(), wrong)) return NO;
    wrong = [WPTestCall() mutableCopy];
    wrong[IOBluetoothHandsFreeCallMultiparty] = @1;
    return !WPCallMatches(WPTestBinding(), wrong);
}
static BOOL WPRetirementMainPathTest(void) {
    WPHfpRetirement serviceFirst = {.serviceClosed = YES, .scoNeedsClosure = YES};
    if (WPRetirementReady(serviceFirst, YES, NO, NO)) return NO;
    serviceFirst.scoClosed = YES;
    if (!WPRetirementReady(serviceFirst, YES, NO, NO)) return NO;

    WPHfpRetirement scoFirst = {.scoNeedsClosure = YES, .scoClosed = YES};
    if (WPRetirementReady(scoFirst, YES, NO, NO)) return NO;
    scoFirst.serviceClosed = YES;
    return WPRetirementReady(scoFirst, YES, NO, NO);
}
static BOOL WPRetirementFailurePathTest(void) {
    WPHfpRetirement pending = {.scoTransferPending = YES, .serviceClosed = YES,
                              .scoNeedsClosure = YES, .scoClosed = YES};
    // Even observed closure and negative properties do not cancel pending open.
    if (WPRetirementReady(pending, YES, NO, NO)) return NO;
    pending.scoTransferPending = NO;
    pending.serviceConnectPending = YES;
    if (WPRetirementReady(pending, YES, NO, NO)) return NO;
    pending.serviceConnectPending = NO;
    if (WPRetirementReady(pending, NO, NO, NO) ||
        WPRetirementReady(pending, YES, YES, NO) ||
        WPRetirementReady(pending, YES, NO, YES)) return NO;
    pending.scoClosed = NO;
    return !WPRetirementReady(pending, YES, NO, NO);
}
int main(void) {
    @autoreleasepool {
        if (!WPMainPathTest() || !WPFailurePathTest()) return 1;
        if (!WPRetirementMainPathTest() || !WPRetirementFailurePathTest()) return 1;
        puts("four synthetic policy tests passed; no native provider executed");
        return 0;
    }
}
#elif defined(WAYPAIR_HFP_OPERATOR_MAIN)
// This executable is an explicitly operator-attested experiment. It does not
// authenticate a Waypair peer or qualify production call audio availability.
static volatile sig_atomic_t WPInterrupted = 0;
static void WPInterrupt(int signalNumber) { (void)signalNumber; WPInterrupted = 1; }
static NSDictionary *WPOperatorBinding;
static NSString *WPOperatorLeasePath;
static int WPOperatorLease(void *context, const char *bindingJSON) {
    (void)context; (void)bindingJSON;
    if (WPInterrupted) return 0;
    NSData *data = [NSData dataWithContentsOfFile:WPOperatorLeasePath];
    if (!data || data.length > 16384) return 0;
    NSDictionary *current = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
    if (![current isKindOfClass:NSDictionary.class]) return 0;
    NSNumber *expiry = current[@"expiresAtUnixMs"];
    NSTimeInterval now = NSDate.date.timeIntervalSince1970 * 1000.0;
    return [expiry isKindOfClass:NSNumber.class] && expiry.doubleValue > now && expiry.doubleValue <= now + 10000.0 &&
        [current[@"binding"] isEqual:WPOperatorBinding] &&
        [current[@"singleActiveCallConfirmed"] isEqual:@YES] &&
        [current[@"trustedPhoneMappingConfirmed"] isEqual:@YES];
}
static void WPOperatorEvent(void *context, const char *json) { (void)context; puts(json); fflush(stdout); }
int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 4 || strcmp(argv[1], "--confirm-native-system-audio-experiment") != 0) {
            fputs("usage: hf-prototype --confirm-native-system-audio-experiment binding.json live-lease.json\n", stderr);
            return 2;
        }
        NSData *bindingData = [NSData dataWithContentsOfFile:[NSString stringWithUTF8String:argv[2]]];
        if (!bindingData || bindingData.length > 8192) return 2;
        WPOperatorBinding = [NSJSONSerialization JSONObjectWithData:bindingData options:0 error:NULL];
        if (!WPBindingValid(WPOperatorBinding)) return 2;
        WPOperatorLeasePath = [NSString stringWithUTF8String:argv[3]];
        NSString *json = [[NSString alloc] initWithData:bindingData encoding:NSUTF8StringEncoding];
        signal(SIGINT, WPInterrupt);
        signal(SIGTERM, WPInterrupt);
        fputs("Operator prototype only: no authenticated app binding, duplex acceptance or handset audibility claim.\n", stderr);
        uint64_t owner = WPHfpStart(json.UTF8String, WPOperatorLease, WPOperatorEvent, NULL);
        if (!owner) return 2;
        int exitCode = 1;
        for (;;) {
            char *snapshot = WPHfpCopySnapshotJSON(owner);
            if (snapshot) {
                NSData *bytes = [NSData dataWithBytes:snapshot length:strlen(snapshot)];
                NSDictionary *state = [NSJSONSerialization JSONObjectWithData:bytes options:0 error:NULL];
                if ([state[@"retired"] isEqual:@YES]) exitCode = [state[@"phase"] isEqual:@"rejected"] ? 3 : 0;
                WPHfpFreeJSON(snapshot);
            }
            if (WPHfpRelease(owner)) break;
            if (WPInterrupted) WPHfpHandback(owner);
            @autoreleasepool { [NSRunLoop.mainRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.1]]; }
        }
        // Exit 0 means native closure observed, never physical audio acceptance.
        return exitCode;
    }
}
#endif

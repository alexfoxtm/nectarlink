// SPDX-License-Identifier: GPL-3.0-or-later
// Read-only facts the app needs around bt_hfp_provider.m, plus the
// user-started actions it may take: listing already-paired phones, asking
// macOS for Bluetooth or microphone access, and checking one chosen phone's
// advertised call-audio support. Nothing here pairs, scans, selects a default
// device or starts capture. The explicit refresh below asks macOS to read the
// one chosen phone's SDP record, which may build a baseband connection to it.
// The app asks for no pairing, scan, unpair, disconnect, prompt, call or audio
// route there; what macOS or the phone does with that link is not up to it.
// Returned strings are malloc'd JSON, freed with WPHfpFreeJSON.
#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreBluetooth/CoreBluetooth.h>
#import <IOBluetooth/IOBluetooth.h>
#import <dispatch/dispatch.h>
#include <stdlib.h>
#include <string.h>

static char *WPSupportCopyJSON(id object) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:object options:NSJSONWritingSortedKeys error:NULL];
    if (!data) return NULL;
    char *json = malloc(data.length + 1);
    if (!json) return NULL;
    memcpy(json, data.bytes, data.length);
    json[data.length] = '\0';
    return json;
}

static BOOL WPSupportHasUsageDescription(NSString *key) {
    id value = [NSBundle.mainBundle objectForInfoDictionaryKey:key];
    return [value isKindOfClass:NSString.class] && [value length] > 0;
}

static NSString *WPSupportBluetoothAuthorization(void) {
    // Without the usage description macOS terminates a process that touches
    // Bluetooth, so the status is not read at all.
    if (!WPSupportHasUsageDescription(@"NSBluetoothAlwaysUsageDescription")) return @"unreadable";
    switch (CBManager.authorization) {
        case CBManagerAuthorizationAllowedAlways: return @"allowed";
        case CBManagerAuthorizationDenied: return @"denied";
        case CBManagerAuthorizationRestricted: return @"restricted";
        case CBManagerAuthorizationNotDetermined: return @"not-determined";
    }
    return @"unreadable";
}

static NSString *WPSupportMicrophoneAuthorization(void) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]) {
        case AVAuthorizationStatusAuthorized: return @"allowed";
        case AVAuthorizationStatusDenied: return @"denied";
        case AVAuthorizationStatusRestricted: return @"restricted";
        case AVAuthorizationStatusNotDetermined: return @"not-determined";
    }
    return @"unreadable";
}

// Status queries only: no manager is created and no access is requested.
char *WPHfpSupportCopyPrerequisitesJSON(void) {
    @autoreleasepool {
        return WPSupportCopyJSON(@{
            @"bluetoothUsageDescription": @(WPSupportHasUsageDescription(@"NSBluetoothAlwaysUsageDescription")),
            @"microphoneUsageDescription": @(WPSupportHasUsageDescription(@"NSMicrophoneUsageDescription")),
            @"bluetooth": WPSupportBluetoothAuthorization(),
            @"microphone": WPSupportMicrophoneAuthorization(),
        });
    }
}

// Only for an explicit user action, with Bluetooth access already granted.
// Lists phones macOS has already paired that advertise the hands-free audio
// gateway service in their cached SDP record. Names are for display; the
// address is the identity.
char *WPHfpSupportCopyPairedGatewaysJSON(void) {
    if (!NSThread.isMainThread || ![WPSupportBluetoothAuthorization() isEqual:@"allowed"]) return NULL;
    @autoreleasepool {
        NSMutableArray *gateways = [NSMutableArray array];
        for (id item in [IOBluetoothDevice pairedDevices] ?: @[]) {
            if (![item isKindOfClass:IOBluetoothDevice.class]) continue;
            IOBluetoothDevice *device = item;
            NSString *address = [[device.addressString stringByReplacingOccurrencesOfString:@":" withString:@"-"]
                uppercaseString];
            if (!device.isPaired || !device.isHandsFreeAudioGateway || address.length != 17) continue;
            [gateways addObject:@{@"address": address, @"name": device.name ?: @""}];
        }
        return WPSupportCopyJSON(gateways);
    }
}

static NSString *WPSupportDeviceString(AudioObjectID device, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress a = {selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    OSStatus result = AudioObjectGetPropertyData(device, &a, 0, NULL, &size, &value);
    if (result != noErr || size != sizeof(value) || !value) {
        if (value) CFRelease(value);
        return nil;
    }
    return CFBridgingRelease(value);
}

static BOOL WPSupportScalar(AudioObjectID object, AudioObjectPropertySelector selector, UInt32 *out) {
    AudioObjectPropertyAddress a = {selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 size = sizeof(*out);
    return AudioObjectGetPropertyData(object, &a, 0, NULL, &size, out) == noErr && size == sizeof(*out);
}

// Same wired/built-in transports the provider accepts for its endpoints.
static BOOL WPSupportLocalTransport(UInt32 transport) {
    return transport == kAudioDeviceTransportTypeBuiltIn || transport == kAudioDeviceTransportTypeUSB ||
        transport == kAudioDeviceTransportTypePCI || transport == kAudioDeviceTransportTypeThunderbolt ||
        transport == kAudioDeviceTransportTypeDisplayPort || transport == kAudioDeviceTransportTypeHDMI ||
        transport == kAudioDeviceTransportTypeFireWire;
}

static id WPSupportDefaultEndpoint(AudioObjectPropertySelector selector) {
    UInt32 device = 0, transport = 0;
    if (!WPSupportScalar(kAudioObjectSystemObject, selector, &device) || !device) return NSNull.null;
    NSString *uid = WPSupportDeviceString(device, kAudioDevicePropertyDeviceUID);
    if (!uid.length) return NSNull.null;
    BOOL local = WPSupportScalar(device, kAudioDevicePropertyTransportType, &transport) &&
        WPSupportLocalTransport(transport);
    return @{@"uid": uid, @"name": WPSupportDeviceString(device, kAudioObjectPropertyName) ?: @"",
             @"local": @(local)};
}

// Read only: the system's current default microphone and speaker.
char *WPHfpSupportCopyDefaultEndpointsJSON(void) {
    @autoreleasepool {
        return WPSupportCopyJSON(@{
            @"input": WPSupportDefaultEndpoint(kAudioHardwarePropertyDefaultInputDevice),
            @"output": WPSupportDefaultEndpoint(kAudioHardwarePropertyDefaultOutputDevice),
        });
    }
}

// No context or borrowed object: the shim supplies a process-lifetime function.
// All notifications arrive on Main; they report a reason to re-read status only.
typedef void (*WPHfpSupportChanged)(void);

@interface WPSupportBluetoothRequestDelegate : NSObject <CBCentralManagerDelegate>
@property(nonatomic, assign) WPHfpSupportChanged authorizationChanged;
@end

@implementation WPSupportBluetoothRequestDelegate
- (void)centralManagerDidUpdateState:(CBCentralManager *)central {
    (void)central;
    if (self.authorizationChanged) self.authorizationChanged();
}
@end

// Retain both for the process lifetime: CBCentralManager's delegate is weak.
// Created only by an explicit request; neither ever scans or connects.
static CBCentralManager *WPSupportBluetoothRequest;
static WPSupportBluetoothRequestDelegate *WPSupportBluetoothDelegate;

// Only for an explicit user action, and only while undecided: creating a
// manager is what makes macOS show its own prompt. Returns 1 when asked.
int WPHfpSupportRequestBluetooth(WPHfpSupportChanged changed) {
    if (!NSThread.isMainThread || !changed || WPSupportBluetoothRequest ||
        ![WPSupportBluetoothAuthorization() isEqual:@"not-determined"]) {
        return 0;
    }
    WPSupportBluetoothDelegate = [WPSupportBluetoothRequestDelegate new];
    WPSupportBluetoothDelegate.authorizationChanged = changed;
    WPSupportBluetoothRequest = [[CBCentralManager alloc] initWithDelegate:WPSupportBluetoothDelegate queue:nil
        options:@{CBCentralManagerOptionShowPowerAlertKey: @NO}];
    return 1;
}

// Only for an explicit user action, and only while undecided: macOS shows
// its own prompt. Returns 1 when that request was made.
int WPHfpSupportRequestMicrophone(WPHfpSupportChanged changed) {
    if (!NSThread.isMainThread || !changed || !WPSupportHasUsageDescription(@"NSMicrophoneUsageDescription") ||
        [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio] != AVAuthorizationStatusNotDetermined) {
        return 0;
    }
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
        // AVFoundation completes on an arbitrary queue. Re-read both statuses
        // on Main rather than treating this result as permission to start audio.
        (void)granted;
        dispatch_async(dispatch_get_main_queue(), ^{ changed(); });
    }];
    return 1;
}

// Lets main-thread provider callbacks run after the app's event loop has
// returned, so quitting can observe the owned handback.
void WPHfpSupportPumpMainRunLoop(double seconds) {
    if (!NSThread.isMainThread || !(seconds > 0.0)) return;
    @autoreleasepool {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds > 0.25 ? 0.25 : seconds, false);
    }
}

// ---- Owner-selected paired-phone SDP refresh (explicit user action only) ----

static NSString *WPSDPAddress(IOBluetoothDevice *device) {
    return [[device.addressString stringByReplacingOccurrencesOfString:@":" withString:@"-"] uppercaseString];
}

// Already-paired phone-class devices, plus anything the strict gateway list
// above accepts (cached hands-free audio gateway) so no strict entry regresses.
// That second clause is the legacy exception: a non-phone device is listed
// only while its cache already reports audio gateway, and such a row is never
// queried, because Rust asks to check only rows no cached read reports as
// audio gateway. Listing and `-begin` apply this same gate.
static BOOL WPSDPEligible(IOBluetoothDevice *device) {
    return device.isPaired
        && (device.deviceClassMajor == kBluetoothDeviceClassMajorPhone || device.isHandsFreeAudioGateway);
}

// Lists eligible phones with their cached SDP/AG facts WITHOUT starting any
// query: main thread, Bluetooth already allowed, side-effect free like the
// gateway list above. `agCached` is the cached SDP answer (YES only when the
// cache currently reports hands-free audio gateway); it is never a grant, and
// NO (unknown) never means unsupported: the cache may simply be cold.
// Class numbers are public IOBluetooth metadata; the address is the identity
// and the name is display-only.
char *WPHfpSupportCopyPairedPhoneCandidatesJSON(void) {
    if (!NSThread.isMainThread || ![WPSupportBluetoothAuthorization() isEqual:@"allowed"]) return NULL;
    @autoreleasepool {
        NSMutableArray *phones = [NSMutableArray array];
        for (id item in [IOBluetoothDevice pairedDevices] ?: @[]) {
            if (![item isKindOfClass:IOBluetoothDevice.class]) continue;
            IOBluetoothDevice *device = item;
            NSString *address = WPSDPAddress(device);
            if (!WPSDPEligible(device) || address.length != 17) continue;
            [phones addObject:@{@"address": address, @"name": device.name ?: @"",
                                @"agCached": @(device.isHandsFreeAudioGateway),
                                @"servicesKnown": @([device getLastServicesUpdate] != nil),
                                @"deviceClassMajor": @(device.deviceClassMajor),
                                @"deviceClassMinor": @(device.deviceClassMinor)}];
        }
        return WPSupportCopyJSON(phones);
    }
}

static BOOL WPSDPValidAddress(NSString *address) {
    if (address.length != 17) return NO;
    NSCharacterSet *nonHex = [[NSCharacterSet characterSetWithCharactersInString:@"0123456789ABCDEF"] invertedSet];
    NSArray *parts = [address componentsSeparatedByString:@"-"];
    if (parts.count != 6) return NO;
    for (NSString *part in parts) {
        if (part.length != 2 || [part rangeOfCharacterFromSet:nonHex].location != NSNotFound) return NO;
    }
    return YES;
}

// One explicit SDP query for one exact eligible already-paired phone. Per the
// public header, `-performSDPQuery:` builds a baseband connection to that
// phone (when not already connected) and calls back when the query completes
// or fails; it names no time bound and no thread, and nothing public cancels
// it. The app makes no inquiry/scan, unpair, closeConnection, permission,
// call, microphone or route request here: only `-performSDPQuery:` on the one
// device, then a re-read of its cached facts.
//
// At most one OS query is outstanding, process-wide. Only that query's actual
// completion, or `-performSDPQuery:` refusing at once, frees the slot; while
// it is held every request answers busy (2) and starts nothing. A query the OS
// never completes keeps the slot busy until the process ends. Every completion
// is reported, once, on Main with its own request; the caller decides whether
// that request is still wanted.
typedef void (*WPSDPRefreshEvent)(uint64_t request, const char *resultJSON);

@class WPSDPRefresh;
static WPSDPRefresh *WPSDPOutstanding;

@interface WPSDPRefresh : NSObject
- (instancetype)initWithAddress:(NSString *)address request:(uint64_t)request event:(WPSDPRefreshEvent)event;
- (BOOL)begin;
- (void)sdpQueryComplete:(IOBluetoothDevice *)device status:(IOReturn)status;
- (void)finishWithDevice:(IOBluetoothDevice *)device status:(IOReturn)status;
@end

@implementation WPSDPRefresh {
    NSString *_address;
    uint64_t _request;
    WPSDPRefreshEvent _event;
    IOBluetoothDevice *_device;
    NSDate *_began;
}

- (instancetype)initWithAddress:(NSString *)address request:(uint64_t)request event:(WPSDPRefreshEvent)event {
    self = [super init];
    if (self) {
        _address = [address copy];
        _request = request;
        _event = event;
    }
    return self;
}

- (BOOL)begin {
    IOBluetoothDevice *device = [IOBluetoothDevice deviceWithAddressString:_address];
    if (!device || !WPSDPEligible(device) || ![_address isEqual:WPSDPAddress(device)]) return NO;
    _device = device;
    _began = [NSDate date];
    return [device performSDPQuery:self] == kIOReturnSuccess;
}

- (void)sdpQueryComplete:(IOBluetoothDevice *)device status:(IOReturn)status {
    // The OS may call back off Main; ownership and the event run on Main.
    __weak WPSDPRefresh *weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        WPSDPRefresh *owner = weakSelf;
        [owner finishWithDevice:device status:status];
    });
}

- (void)finishWithDevice:(IOBluetoothDevice *)device status:(IOReturn)status {
    if (!NSThread.isMainThread || WPSDPOutstanding != self) return;
    WPSDPOutstanding = nil;
    BOOL ok = device && status == kIOReturnSuccess && [_address isEqual:WPSDPAddress(device)];
    // Known only when this query refreshed the record: a missing, older or
    // coarser stamp reads as not known, which is never reported unsupported.
    NSDate *update = ok ? [device getLastServicesUpdate] : nil;
    BOOL known = update && [update compare:_began] != NSOrderedAscending;
    BOOL ag = ok && device.isHandsFreeAudioGateway;
    NSData *data = [NSJSONSerialization dataWithJSONObject:@{@"address": _address, @"ok": @(ok),
        @"agSupported": @(ag), @"servicesKnown": @(known)}
        options:NSJSONWritingSortedKeys error:NULL];
    NSString *json = data ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : @"{}";
    WPSDPRefreshEvent event = _event;
    _event = NULL;
    _device = nil;
    if (event) event(_request, json.UTF8String);
}

@end

// 1 started, 2 busy (an earlier query is still outstanding; nothing started),
// 0 refused. Main thread, explicit user action only.
int WPHfpSupportRefreshPhoneSDP(const char *address, uint64_t request, WPSDPRefreshEvent event) {
    if (!NSThread.isMainThread || !address || request == 0 || !event) return 0;
    if (WPSDPOutstanding) return 2;
    if (![WPSupportBluetoothAuthorization() isEqual:@"allowed"]) return 0;
    NSString *candidate = [[NSString stringWithUTF8String:address] uppercaseString];
    if (!WPSDPValidAddress(candidate)) return 0;
    WPSDPRefresh *owner = [[WPSDPRefresh alloc] initWithAddress:candidate request:request event:event];
    if (![owner begin]) return 0;
    WPSDPOutstanding = owner;
    return 1;
}

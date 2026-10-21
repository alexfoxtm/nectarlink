// SPDX-License-Identifier: GPL-3.0-or-later
// Read-only facts the app needs around bt_hfp_provider.m, plus the
// user-started actions it may take: listing already-paired phones and asking
// macOS for Bluetooth or microphone access. Nothing here pairs, scans,
// connects, selects a default device or starts capture. Returned strings are
// malloc'd JSON, freed with WPHfpFreeJSON.
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

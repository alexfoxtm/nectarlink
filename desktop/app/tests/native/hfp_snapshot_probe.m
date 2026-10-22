// Actual production JSON contracts; exactly false/nil-HF and true/fake-HF states.
// No start, manager, SDP, permission request, paired-device read or live HAL read.
#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreBluetooth/CoreBluetooth.h>
#import <IOBluetooth/IOBluetooth.h>
#import <objc/runtime.h>
#include <stdio.h>
#include <string.h>

static BOOL WPProbeFlag;
static NSUInteger WPProbePairedCalls, WPProbeAuthorizationCalls, WPProbeUsageCalls;
static NSUInteger WPProbeAudioSizeCalls, WPProbeAudioDataCalls;
static NSBundle *WPProbeBundle;
static NSObject *WPProbePhone;
static const AudioDeviceID WPProbeDevice = 0xF00D;
static NSString *const WPProbeUID = @"synthetic-input-uid";

static void WPProbeRequire(BOOL ok, NSString *reason) {
    if (!ok) @throw [NSException exceptionWithName:@"WPProbeFailure" reason:reason userInfo:nil];
}

static void WPProbeAudioArguments(AudioObjectID object, const AudioObjectPropertyAddress *address,
                                  UInt32 qualifierSize, const void *qualifier) {
    WPProbeRequire(object == WPProbeDevice && address && !qualifierSize && !qualifier &&
                   address->mElement == kAudioObjectPropertyElementMain, @"unexpected HAL arguments");
}

static OSStatus WPProbeAudioDataSize(AudioObjectID object, const AudioObjectPropertyAddress *address,
                                    UInt32 qualifierSize, const void *qualifier, UInt32 *size) {
    WPProbeAudioArguments(object, address, qualifierSize, qualifier);
    WPProbeRequire(size && address->mSelector == kAudioDevicePropertyStreams &&
                   address->mScope == kAudioObjectPropertyScopeInput, @"unexpected HAL size query");
    ++WPProbeAudioSizeCalls;
    *size = sizeof(AudioStreamID);
    return noErr;
}

static OSStatus WPProbeAudioData(AudioObjectID object, const AudioObjectPropertyAddress *address,
                                UInt32 qualifierSize, const void *qualifier, UInt32 *size, void *out) {
    WPProbeAudioArguments(object, address, qualifierSize, qualifier);
    WPProbeRequire(size && out, @"missing HAL output");
    ++WPProbeAudioDataCalls;
    if (address->mSelector == kAudioDevicePropertyStreams) {
        WPProbeRequire(address->mScope == kAudioObjectPropertyScopeInput && *size == sizeof(AudioStreamID),
                       @"unexpected stream query");
        AudioStreamID stream = 0xCAFE;
        memcpy(out, &stream, sizeof(stream));
        return noErr;
    }
    WPProbeRequire(address->mScope == kAudioObjectPropertyScopeGlobal, @"unexpected scalar/UID scope");
    if (address->mSelector == kAudioDevicePropertyDeviceUID) {
        WPProbeRequire(*size == sizeof(CFStringRef), @"unexpected UID size");
        CFStringRef uid = CFStringCreateCopy(kCFAllocatorDefault, (__bridge CFStringRef)WPProbeUID);
        WPProbeRequire(uid != NULL, @"synthetic UID allocation failed");
        memcpy(out, &uid, sizeof(uid)); // Production WPUID consumes this +1 reference.
        return noErr;
    }
    WPProbeRequire(*size == sizeof(UInt32), @"unexpected scalar size");
    UInt32 value = 0;
    switch (address->mSelector) {
        case kAudioDevicePropertyDeviceIsAlive: value = 1; break;
        case kAudioDevicePropertyDeviceIsRunning: value = WPProbeFlag ? 1 : 0; break;
        case kAudioDevicePropertyTransportType: value = kAudioDeviceTransportTypeBuiltIn; break;
        default: WPProbeRequire(NO, @"unexpected HAL selector; no live fallback");
    }
    memcpy(out, &value, sizeof(value));
    return noErr;
}

// Framework declarations are already imported: replace only calls in this TU.
#define AudioObjectGetPropertyDataSize WPProbeAudioDataSize
#define AudioObjectGetPropertyData WPProbeAudioData
#import "../../src/mac/bt_hfp_support.m"
#import "../../src/mac/bt_hfp_provider.m"
#undef AudioObjectGetPropertyData
#undef AudioObjectGetPropertyDataSize

@interface WPFakeHF : NSObject
@property(nonatomic) BOOL connected;
@property(nonatomic) BOOL scoConnected;
- (BOOL)isConnected;
- (BOOL)isSCOConnected;
@end
@implementation WPFakeHF
- (BOOL)isConnected { return self.connected; }
- (BOOL)isSCOConnected { return self.scoConnected; }
- (void)doesNotRecognizeSelector:(SEL)selector {
    WPProbeRequire(NO, [@"unexpected fake HF selector: " stringByAppendingString:NSStringFromSelector(selector)]);
}
@end

@interface WPFakePhone : NSObject
@end
@implementation WPFakePhone
- (BOOL)isKindOfClass:(Class)cls {
    WPProbeRequire(cls == IOBluetoothDevice.class, @"unexpected fake phone class query");
    return YES;
}
- (BOOL)isPaired { return YES; }
- (BluetoothDeviceClassMajor)deviceClassMajor { return kBluetoothDeviceClassMajorPhone; }
- (BluetoothDeviceClassMinor)deviceClassMinor { return 0; }
- (NSString *)addressString { return @"00-11-22-33-44-55"; }
- (NSString *)name { return @"Synthetic HFP probe phone"; }
- (BOOL)isHandsFreeAudioGateway { return WPProbeFlag; }
- (NSDate *)getLastServicesUpdate {
    return WPProbeFlag ? [NSDate dateWithTimeIntervalSince1970:1] : nil;
}
- (void)doesNotRecognizeSelector:(SEL)selector {
    WPProbeRequire(NO, [@"unexpected fake phone selector: " stringByAppendingString:NSStringFromSelector(selector)]);
}
@end

static NSArray *WPProbePairedDevices(id receiver, SEL selector) {
    WPProbeRequire(receiver == IOBluetoothDevice.class && selector == @selector(pairedDevices),
                   @"unexpected paired-device call");
    ++WPProbePairedCalls;
    return @[WPProbePhone];
}
static CBManagerAuthorization WPProbeAuthorization(id receiver, SEL selector) {
    WPProbeRequire(receiver == CBManager.class && selector == @selector(authorization), @"unexpected authorization call");
    ++WPProbeAuthorizationCalls;
    return CBManagerAuthorizationAllowedAlways;
}
static id WPProbeUsage(id receiver, SEL selector, NSString *key) {
    WPProbeRequire(receiver == WPProbeBundle && selector == @selector(objectForInfoDictionaryKey:) &&
                   [key isEqual:@"NSBluetoothAlwaysUsageDescription"], @"unexpected bundle usage lookup");
    ++WPProbeUsageCalls;
    return @"Synthetic usage description for this disposable probe";
}
static void WPProbeBoolean(id value, BOOL expected, NSString *label) {
    WPProbeRequire([value isKindOfClass:NSNumber.class] &&
                   CFGetTypeID((__bridge CFTypeRef)value) == CFBooleanGetTypeID() && [value boolValue] == expected,
                   [@"not the expected CFBoolean: " stringByAppendingString:label]);
}

int main(void) { @autoreleasepool {
    int result = 0;
    Method paired = NULL, authorization = NULL, usage = NULL;
    IMP originalPaired = NULL, originalAuthorization = NULL, originalUsage = NULL;
    NSMutableArray *rows = [NSMutableArray array];
    @try {
        WPProbeRequire(NSThread.isMainThread, @"probe must run on main");
        WPProbeBundle = NSBundle.mainBundle;
        WPProbePhone = [WPFakePhone new];
        paired = class_getClassMethod(IOBluetoothDevice.class, @selector(pairedDevices));
        authorization = class_getClassMethod(CBManager.class, @selector(authorization));
        usage = class_getInstanceMethod(object_getClass(WPProbeBundle), @selector(objectForInfoDictionaryKey:));
        WPProbeRequire(paired && authorization && usage, @"required public method not found");
        originalPaired = method_setImplementation(paired, (IMP)WPProbePairedDevices);
        originalAuthorization = method_setImplementation(authorization, (IMP)WPProbeAuthorization);
        originalUsage = method_setImplementation(usage, (IMP)WPProbeUsage);
        for (NSNumber *flag in @[@NO, @YES]) {
            WPProbeFlag = flag.boolValue;
            WPProbePairedCalls = WPProbeAuthorizationCalls = WPProbeUsageCalls = 0;
            WPProbeAudioSizeCalls = WPProbeAudioDataCalls = 0;
            WPHfpProvider *p = [[WPHfpProvider alloc] initWithBinding:@{} json:@"{}" lease:NULL event:NULL context:NULL];
            if (WPProbeFlag) {
                WPFakeHF *hf = [WPFakeHF new]; hf.connected = YES; hf.scoConnected = YES;
                [p setValue:hf forKey:@"hf"];
            }
            NSDictionary *snapshot = p.snapshot;
            WPProbeBoolean(snapshot[@"slcObserved"], WPProbeFlag, @"slcObserved before JSON");
            WPProbeBoolean(snapshot[@"scoObserved"], WPProbeFlag, @"scoObserved before JSON");
            char *raw = WPHfpSupportCopyPairedPhoneCandidatesJSON();
            NSData *candidateData = raw ? [NSData dataWithBytes:raw length:strlen(raw)] : nil;
            WPHfpFreeJSON(raw);
            WPProbeRequire(candidateData != nil, @"production paired-phone JSON is missing");
            NSArray *candidates = [NSJSONSerialization JSONObjectWithData:candidateData options:0 error:NULL];
            WPProbeRequire([candidates isKindOfClass:NSArray.class] && candidates.count == 1 &&
                           [candidates[0][@"address"] isEqual:@"00-11-22-33-44-55"], @"unexpected paired-phone JSON");
            WPProbeBoolean(candidates[0][@"servicesKnown"], WPProbeFlag, @"servicesKnown from production JSON");
            NSDictionary *endpoint = WPEndpoint(WPProbeDevice, kAudioObjectPropertyScopeInput, WPProbeUID);
            WPProbeRequire(endpoint != nil, @"production endpoint rejected synthetic HAL facts");
            WPProbeBoolean(endpoint[@"running"], WPProbeFlag, @"running before JSON");
            WPProbeRequire(WPProbePairedCalls == 1 && WPProbeAuthorizationCalls == 1 && WPProbeUsageCalls == 1 &&
                           WPProbeAudioSizeCalls == 1 && WPProbeAudioDataCalls == 5, @"unexpected fixture call count");
            NSDictionary *payload = @{@"snapshot":snapshot, @"pairedPhones":candidates, @"endpoint":endpoint};
            NSData *wire = [NSJSONSerialization dataWithJSONObject:payload options:NSJSONWritingSortedKeys error:NULL];
            WPProbeRequire(wire != nil, @"production values did not encode as JSON");
            NSDictionary *decoded = [NSJSONSerialization JSONObjectWithData:wire options:0 error:NULL];
            WPProbeRequire([decoded isKindOfClass:NSDictionary.class], @"JSON round trip did not yield an object");
            WPProbeBoolean(decoded[@"snapshot"][@"slcObserved"], WPProbeFlag, @"slcObserved after JSON");
            WPProbeBoolean(decoded[@"snapshot"][@"scoObserved"], WPProbeFlag, @"scoObserved after JSON");
            WPProbeBoolean(decoded[@"pairedPhones"][0][@"servicesKnown"], WPProbeFlag, @"servicesKnown after JSON");
            WPProbeBoolean(decoded[@"endpoint"][@"running"], WPProbeFlag, @"running after JSON");
            [rows addObject:@{@"fixtureConnected":flag, @"production":decoded}];
        }
    } @catch (NSException *error) {
        fprintf(stderr, "HFP JSON contract probe failed: %s\n", error.reason.UTF8String);
        result = 2;
    } @finally {
        if (originalUsage) method_setImplementation(usage, originalUsage);
        if (originalAuthorization) method_setImplementation(authorization, originalAuthorization);
        if (originalPaired) method_setImplementation(paired, originalPaired);
        WPProbePhone = nil; WPProbeBundle = nil;
    }
    if (!result) {
        NSData *data = [NSJSONSerialization dataWithJSONObject:rows options:NSJSONWritingSortedKeys error:NULL];
        if (!data || rows.count != 2) return 2;
        fwrite(data.bytes, 1, data.length, stdout); fputc('\n', stdout);
    }
    return result;
} }

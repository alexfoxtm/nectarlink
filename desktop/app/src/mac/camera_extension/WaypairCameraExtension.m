// SPDX-License-Identifier: GPL-3.0-or-later
// Separate public CoreMediaIO Camera Extension executable (macOS 12.3+).
// No window server, local camera, phone transport, listener socket or child process.
#import <Foundation/Foundation.h>
#import <CoreMediaIO/CoreMediaIO.h>
#import <CoreVideo/CoreVideo.h>
#import <Security/Security.h>
#include <string.h>
#include "WaypairCamera.h"

static NSError *wpError(NSString *message) {
    return [NSError errorWithDomain:@"com.waypair.camera" code:1
                           userInfo:@{NSLocalizedDescriptionKey: message}];
}
static uint64_t hostNS(void) {
    return (uint64_t)CMTimeConvertScale(CMClockGetTime(CMClockGetHostTimeClock()),
        1000000000, kCMTimeRoundingMethod_Default).value;
}
static NSDictionary *durationDictionary(void) {
    return CFBridgingRelease(CMTimeCopyAsDictionary(CMTimeMake(1, WP_CAMERA_FPS), kCFAllocatorDefault));
}

@class WPDevice;
@interface WPStream : NSObject <CMIOExtensionStreamSource> {
    NSArray<CMIOExtensionStreamFormat *> *_formats;
}
@property(nonatomic, strong) CMIOExtensionStream *stream;
@property(nonatomic, weak) WPDevice *owner;
@property(nonatomic) BOOL sink;
@property(atomic, readonly) NSArray<CMIOExtensionStreamFormat *> *formats;
- (instancetype)initWithFormat:(CMIOExtensionStreamFormat *)format;
@end

@interface WPDevice : NSObject <CMIOExtensionDeviceSource>
@property(nonatomic, strong) CMIOExtensionDevice *device;
@property(nonatomic, strong) WPStream *source;
@property(nonatomic, strong) WPStream *sink;
@property(nonatomic, strong) CMIOExtensionClient *producer;
@property(nonatomic, strong) dispatch_queue_t queue;
@property(nonatomic, strong) dispatch_source_t timer;
@property(nonatomic) NSUInteger sourceStarts;
@property(nonatomic) BOOL sinkRunning;
@property(nonatomic) BOOL consuming;
@property(nonatomic) uint64_t generation;
@property(nonatomic) CVPixelBufferRef latest;
@property(nonatomic) uint64_t latestTime;
@property(nonatomic) CVPixelBufferRef black;
@property(nonatomic) CMVideoFormatDescriptionRef format;
- (instancetype)initWithQueue:(dispatch_queue_t)queue error:(NSError **)error;
- (BOOL)authorizeProducer:(CMIOExtensionClient *)client;
- (void)stopProducer;
- (void)disconnect:(CMIOExtensionClient *)client;
- (void)ensureTimer;
- (void)tick;
@end

@implementation WPStream
- (instancetype)initWithFormat:(CMIOExtensionStreamFormat *)format {
    self = [super init];
    if (self) _formats = @[format];
    return self;
}
- (NSArray<CMIOExtensionStreamFormat *> *)formats { return _formats; }
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    NSMutableSet *properties = [NSMutableSet setWithArray:@[
        CMIOExtensionPropertyStreamActiveFormatIndex, CMIOExtensionPropertyStreamFrameDuration,
        CMIOExtensionPropertyStreamMaxFrameDuration]];
    if (self.sink) [properties addObjectsFromArray:@[
        CMIOExtensionPropertyStreamSinkBufferQueueSize,
        CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup,
        CMIOExtensionPropertyStreamSinkBufferUnderrunCount,
        CMIOExtensionPropertyStreamSinkEndOfData]];
    return properties;
}
- (CMIOExtensionStreamProperties *)streamPropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                         error:(NSError **)error {
    (void)error;
    CMIOExtensionStreamProperties *p = [[CMIOExtensionStreamProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyStreamActiveFormatIndex]) p.activeFormatIndex = @0;
    if ([requested containsObject:CMIOExtensionPropertyStreamFrameDuration]) p.frameDuration = durationDictionary();
    if ([requested containsObject:CMIOExtensionPropertyStreamMaxFrameDuration]) p.maxFrameDuration = durationDictionary();
    if (self.sink) {
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBufferQueueSize]) p.sinkBufferQueueSize = @2;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup]) p.sinkBuffersRequiredForStartup = @1;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBufferUnderrunCount]) p.sinkBufferUnderrunCount = @0;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkEndOfData]) p.sinkEndOfData = @(!self.owner.sinkRunning);
    }
    return p;
}
- (BOOL)setStreamProperties:(CMIOExtensionStreamProperties *)properties error:(NSError **)error {
    // Exactly one negotiated format and frame rate in this first slice.
    if ((properties.activeFormatIndex && properties.activeFormatIndex.unsignedIntegerValue != 0) ||
        (properties.frameDuration && CMTimeCompare(CMTimeMakeFromDictionary(
            (__bridge CFDictionaryRef)properties.frameDuration), CMTimeMake(1, WP_CAMERA_FPS)) != 0) ||
        (properties.maxFrameDuration && CMTimeCompare(CMTimeMakeFromDictionary(
            (__bridge CFDictionaryRef)properties.maxFrameDuration), CMTimeMake(1, WP_CAMERA_FPS)) != 0)) {
        if (error) *error = wpError(@"Only 1280×720 at 30 fps is available.");
        return NO;
    }
    if (self.sink && properties.sinkEndOfData.boolValue) [self.owner stopProducer];
    return YES;
}
- (BOOL)authorizedToStartStreamForClient:(CMIOExtensionClient *)client {
    // Source consumers remain subject to macOS camera privacy mediation.
    return self.sink ? [self.owner authorizeProducer:client] : YES;
}
- (BOOL)startStreamAndReturnError:(NSError **)error {
    if (self.sink) {
        if (!self.owner.producer || self.owner.sinkRunning) {
            if (error) *error = wpError(@"No exclusive authorized producer.");
            return NO;
        }
        self.owner.sinkRunning = YES;
        self.owner.generation += 1;
    } else {
        self.owner.sourceStarts += 1;
    }
    [self.owner ensureTimer];
    return YES;
}
- (BOOL)stopStreamAndReturnError:(NSError **)error {
    (void)error;
    if (self.sink) [self.owner stopProducer];
    else if (self.owner.sourceStarts > 0) {
        self.owner.sourceStarts -= 1;
        if (self.owner.sourceStarts == 0 && self.owner.latest) {
            CVPixelBufferRelease(self.owner.latest); self.owner.latest = NULL;
        }
    }
    return YES;
}
@end

@implementation WPDevice
- (instancetype)initWithQueue:(dispatch_queue_t)queue error:(NSError **)error {
    self = [super init];
    if (!self) return nil;
    _queue = queue;
    OSStatus status = CMVideoFormatDescriptionCreate(kCFAllocatorDefault, kCVPixelFormatType_32BGRA,
        WP_CAMERA_WIDTH, WP_CAMERA_HEIGHT, NULL, &_format);
    NSDictionary *attrs = @{(id)kCVPixelBufferIOSurfacePropertiesKey: @{}};
    if (status == noErr) status = CVPixelBufferCreate(kCFAllocatorDefault,
        WP_CAMERA_WIDTH, WP_CAMERA_HEIGHT, kCVPixelFormatType_32BGRA,
        (__bridge CFDictionaryRef)attrs, &_black);
    if (status != noErr || !_black) {
        if (error) *error = wpError(@"Could not allocate camera format/black buffer.");
        return nil;
    }
    if (CVPixelBufferLockBaseAddress(_black, 0) != kCVReturnSuccess) {
        if (error) *error = wpError(@"Could not lock camera black buffer.");
        return nil;
    }
    size_t pitch = CVPixelBufferGetBytesPerRow(_black);
    uint8_t *pixels = CVPixelBufferGetBaseAddress(_black);
    memset(pixels, 0, pitch * WP_CAMERA_HEIGHT);
    for (size_t y = 0; y < WP_CAMERA_HEIGHT; ++y)
        for (size_t x = 0; x < WP_CAMERA_WIDTH; ++x) pixels[y * pitch + x * 4 + 3] = 255;
    CVPixelBufferUnlockBaseAddress(_black, 0);
    _device = [[CMIOExtensionDevice alloc] initWithLocalizedName:@"Waypair Camera"
        deviceID:[[NSUUID alloc] initWithUUIDString:@WP_CAMERA_DEVICE_UUID] legacyDeviceID:nil source:self];
    CMTime duration = CMTimeMake(1, WP_CAMERA_FPS);
    CMIOExtensionStreamFormat *format = [[CMIOExtensionStreamFormat alloc]
        initWithFormatDescription:_format maxFrameDuration:duration minFrameDuration:duration
        validFrameDurations:nil];
    _source = [[WPStream alloc] initWithFormat:format]; _source.owner = self;
    _source.stream = [[CMIOExtensionStream alloc] initWithLocalizedName:@"Waypair Camera"
        streamID:[[NSUUID alloc] initWithUUIDString:@WP_CAMERA_SOURCE_UUID]
        direction:CMIOExtensionStreamDirectionSource clockType:CMIOExtensionStreamClockTypeHostTime source:_source];
    _sink = [[WPStream alloc] initWithFormat:format]; _sink.owner = self; _sink.sink = YES;
    _sink.stream = [[CMIOExtensionStream alloc] initWithLocalizedName:@"Waypair Camera Producer"
        streamID:[[NSUUID alloc] initWithUUIDString:@WP_CAMERA_SINK_UUID]
        direction:CMIOExtensionStreamDirectionSink clockType:CMIOExtensionStreamClockTypeHostTime source:_sink];
    if (![_device addStream:_source.stream error:error] || ![_device addStream:_sink.stream error:error]) return nil;
    return self;
}
- (void)dealloc {
    if (_timer) dispatch_source_cancel(_timer);
    if (_latest) CVPixelBufferRelease(_latest);
    if (_black) CVPixelBufferRelease(_black);
    if (_format) CFRelease(_format);
}
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    return [NSSet setWithObject:CMIOExtensionPropertyDeviceModel];
}
- (CMIOExtensionDeviceProperties *)devicePropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                         error:(NSError **)error {
    (void)error;
    CMIOExtensionDeviceProperties *p = [[CMIOExtensionDeviceProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyDeviceModel]) p.model = @"Waypair Phone Camera";
    return p;
}
- (BOOL)setDeviceProperties:(CMIOExtensionDeviceProperties *)properties error:(NSError **)error {
    (void)properties;
    if (error) *error = wpError(@"Camera device properties are read-only.");
    return NO;
}
- (BOOL)authorizeProducer:(CMIOExtensionClient *)client {
    if (_producer) return [_producer.clientID isEqual:client.clientID] && !_sinkRunning;
    // Never accept any app just because it knows the public device UUID.
    // Packaging supplies the exact host designated requirement (bundle + team).
    NSString *rule = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"WaypairProducerRequirement"];
    if (![rule isKindOfClass:NSString.class] || !rule.length || [rule containsString:@"REPLACE_"]) return NO;
    SecRequirementRef requirement = NULL;
    SecCodeRef code = NULL;
    OSStatus status = SecRequirementCreateWithString((__bridge CFStringRef)rule,
        kSecCSDefaultFlags, &requirement);
    NSDictionary *guest = @{(id)kSecGuestAttributePid: @(client.pid)};
    if (status == errSecSuccess) status = SecCodeCopyGuestWithAttributes(NULL,
        (__bridge CFDictionaryRef)guest, kSecCSDefaultFlags, &code);
    if (status == errSecSuccess) status = SecCodeCheckValidity(code, kSecCSStrictValidate, requirement);
    if (code) CFRelease(code);
    if (requirement) CFRelease(requirement);
    if (status != errSecSuccess) return NO;
    _producer = client;
    return YES;
}
- (void)stopProducer {
    _sinkRunning = NO;
    _generation += 1;
    _consuming = NO;
    _producer = nil;
    _latestTime = 0;
    if (_latest) { CVPixelBufferRelease(_latest); _latest = NULL; }
    // Existing source clients receive black on the next 30 Hz tick.
}
- (void)disconnect:(CMIOExtensionClient *)client {
    if ([_producer.clientID isEqual:client.clientID]) [self stopProducer];
}
- (void)ensureTimer {
    if (_timer) return;
    _timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, _queue);
    dispatch_source_set_timer(_timer, DISPATCH_TIME_NOW, NSEC_PER_SEC / WP_CAMERA_FPS, NSEC_PER_MSEC);
    __weak WPDevice *weakSelf = self;
    dispatch_source_set_event_handler(_timer, ^{ [weakSelf tick]; });
    dispatch_resume(_timer);
}
- (void)tick {
    if (!_sinkRunning && !_sourceStarts) {
        dispatch_source_cancel(_timer); _timer = nil;
        return;
    }
    uint64_t now = hostNS();
    if (_latest && (now < _latestTime || now - _latestTime > WP_CAMERA_MAX_AGE_NS)) {
        CVPixelBufferRelease(_latest); _latest = NULL; _latestTime = 0;
    }
    if (_sinkRunning && !_consuming && _producer) {
        _consuming = YES;
        uint64_t generation = _generation;
        NSUUID *clientID = _producer.clientID;
        __weak WPDevice *weakSelf = self;
        [_sink.stream consumeSampleBufferFromClient:_producer completionHandler:
            ^(CMSampleBufferRef sample, uint64_t sequence, CMIOExtensionStreamDiscontinuityFlags discontinuity,
              BOOL hasMore, NSError *error) {
            (void)discontinuity; (void)hasMore;
            // The API owns this borrowed buffer during its completion; retain
            // across the hop to our one state/camera queue only.
            if (sample) CFRetain(sample);
            WPDevice *owner = weakSelf;
            if (!owner) { if (sample) CFRelease(sample); return; }
            dispatch_async(owner.queue, ^{
                if (owner.generation != generation || !owner.sinkRunning ||
                    ![owner.producer.clientID isEqual:clientID]) {
                    if (sample) CFRelease(sample);
                    return;
                }
                owner.consuming = NO;
                if (error) { [owner stopProducer]; if (sample) CFRelease(sample); return; }
                if (sample) {
                    CVPixelBufferRef image = CMSampleBufferGetImageBuffer(sample);
                    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample);
                    uint64_t now = hostNS();
                    int64_t ptsNS = CMTIME_IS_NUMERIC(pts) ? CMTimeConvertScale(pts,
                        1000000000, kCMTimeRoundingMethod_Default).value : -1;
                    BOOL valid = image && CVPixelBufferGetPixelFormatType(image) == kCVPixelFormatType_32BGRA &&
                        CVPixelBufferGetWidth(image) == WP_CAMERA_WIDTH && CVPixelBufferGetHeight(image) == WP_CAMERA_HEIGHT &&
                        ptsNS >= 0 && (uint64_t)ptsNS <= now && now - (uint64_t)ptsNS <= WP_CAMERA_MAX_AGE_NS;
                    if (valid && owner.sourceStarts) {
                        CVPixelBufferRetain(image);
                        if (owner.latest) CVPixelBufferRelease(owner.latest);
                        owner.latest = image; owner.latestTime = (uint64_t)ptsNS;
                    }
                    [owner.sink.stream notifyScheduledOutputChanged:[[CMIOExtensionScheduledOutput alloc]
                        initWithSequenceNumber:sequence hostTimeInNanoseconds:now]];
                    CFRelease(sample);
                }
            });
        }];
    }
    if (!_sourceStarts) return;
    CMSampleTimingInfo timing = {CMTimeMake(1, WP_CAMERA_FPS), CMTimeMake((int64_t)now, 1000000000), kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    if (CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, _latest ?: _black,
        _format, &timing, &sample) == noErr && sample) {
        [_source.stream sendSampleBuffer:sample discontinuity:0 hostTimeInNanoseconds:now];
        CFRelease(sample);
    }
}
@end

@interface WPProvider : NSObject <CMIOExtensionProviderSource>
@property(nonatomic, strong) CMIOExtensionProvider *provider;
@property(nonatomic, strong) WPDevice *camera;
@end
@implementation WPProvider
- (instancetype)init {
    self = [super init];
    if (!self) return nil;
    dispatch_queue_t queue = dispatch_queue_create("com.waypair.camera.frames", DISPATCH_QUEUE_SERIAL);
    _provider = [[CMIOExtensionProvider alloc] initWithSource:self clientQueue:queue];
    NSError *error = nil;
    _camera = [[WPDevice alloc] initWithQueue:queue error:&error];
    if (!_camera || ![_provider addDevice:_camera.device error:&error]) return nil;
    return self;
}
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    return [NSSet setWithArray:@[CMIOExtensionPropertyProviderName, CMIOExtensionPropertyProviderManufacturer]];
}
- (CMIOExtensionProviderProperties *)providerPropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                             error:(NSError **)error {
    (void)error;
    CMIOExtensionProviderProperties *p = [[CMIOExtensionProviderProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyProviderName]) p.name = @"Waypair Camera";
    if ([requested containsObject:CMIOExtensionPropertyProviderManufacturer]) p.manufacturer = @"Waypair";
    return p;
}
- (BOOL)setProviderProperties:(CMIOExtensionProviderProperties *)properties error:(NSError **)error {
    (void)properties;
    if (error) *error = wpError(@"Provider properties are read-only.");
    return NO;
}
- (BOOL)connectClient:(CMIOExtensionClient *)client error:(NSError **)error {
    (void)client; (void)error; return YES;
}
- (void)disconnectClient:(CMIOExtensionClient *)client { [_camera disconnect:client]; }
@end

int main(void) {
    @autoreleasepool {
        __attribute__((objc_precise_lifetime)) WPProvider *source = [WPProvider new];
        if (!source) return 1;
        [CMIOExtensionProvider startServiceWithProvider:source.provider];
        CFRunLoopRun();
    }
    return 0;
}

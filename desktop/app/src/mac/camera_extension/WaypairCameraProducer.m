// SPDX-License-Identifier: GPL-3.0-or-later
// Compiles into the existing Rust/Qt application, not into a replacement app.
#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <CoreMediaIO/CoreMediaIO.h>
#import <CoreVideo/CoreVideo.h>
#import <Accelerate/Accelerate.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "WaypairCamera.h"

struct WPCameraProducer {
    CMIODeviceID device;
    CMIOStreamID sink;
    CMSimpleQueueRef queue;
    CVPixelBufferPoolRef pool;
    CMVideoFormatDescriptionRef format;
    int64_t lastPTS;
    uint32_t width, height;
    BOOL started;
    CFStringRef providerUID;
};

static CMIOObjectPropertyAddress address(CMIOObjectPropertySelector selector,
                                         CMIOObjectPropertyScope scope) {
    return (CMIOObjectPropertyAddress){selector, scope, kCMIOObjectPropertyElementMain};
}

static BOOL copyIDs(CMIOObjectID object, CMIOObjectPropertyAddress a,
                    UInt32 **out, UInt32 *count) {
    UInt32 size = 0;
    if (CMIOObjectGetPropertyDataSize(object, &a, 0, NULL, &size) != noErr ||
        size == 0 || size % sizeof(UInt32) != 0 || size > 65536) return NO;
    UInt32 *ids = calloc(1, size);
    if (!ids) return NO;
    UInt32 used = 0;
    if (CMIOObjectGetPropertyData(object, &a, 0, NULL, size, &used, ids) != noErr ||
        used > size || used % sizeof(UInt32) != 0) { free(ids); return NO; }
    *out = ids; *count = used / sizeof(UInt32); return YES;
}

static BOOL deviceUIDMatches(CMIODeviceID device) {
    CMIOObjectPropertyAddress a = address(kCMIODevicePropertyDeviceUID, kCMIOObjectPropertyScopeGlobal);
    CFStringRef uid = NULL;
    UInt32 used = 0;
    OSStatus status = CMIOObjectGetPropertyData(device, &a, 0, NULL, sizeof(uid), &used, &uid);
    BOOL matches = status == noErr && uid &&
        CFEqual(uid, CFSTR(WP_CAMERA_DEVICE_UUID));
    if (uid) CFRelease(uid);
    return matches;
}

static CFTypeRef copyProperty(CMIOObjectID object, CMIOObjectPropertySelector selector,
                              CFTypeID expectedType);

// The original prototype uses the same UUIDs with a 720p-only sink. Never
// authorize a UHD producer against that older installed protocol.
static BOOL nativeFormatMatches(CMFormatDescriptionRef format, NSUInteger index) {
    if (!format || CFGetTypeID(format) != CMFormatDescriptionGetTypeID() ||
        CMFormatDescriptionGetMediaType(format) != kCMMediaType_Video ||
        CMFormatDescriptionGetMediaSubType(format) != kCVPixelFormatType_32BGRA) return NO;
    const int widths[] = {1280, 1920, WP_CAMERA_WIDTH};
    const int heights[] = {720, 1080, WP_CAMERA_HEIGHT};
    CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format);
    return index < 3 && size.width == widths[index] && size.height == heights[index];
}
static BOOL nativeStreamMatches(CMIOStreamID stream, BOOL sink) {
    CFArrayRef formats = (CFArrayRef)copyProperty(stream, kCMIOStreamPropertyFormatDescriptions,
                                                 CFArrayGetTypeID());
    if (!formats) return NO;
    BOOL valid = CFArrayGetCount(formats) == (sink ? 1 : 3);
    for (CFIndex i = 0; valid && i < CFArrayGetCount(formats); ++i)
        valid = nativeFormatMatches((CMFormatDescriptionRef)CFArrayGetValueAtIndex(formats, i), sink ? 2 : i);
    CFRelease(formats);
    CMFormatDescriptionRef current = (CMFormatDescriptionRef)copyProperty(stream,
        kCMIOStreamPropertyFormatDescription, CMFormatDescriptionGetTypeID());
    BOOL currentValid = sink ? nativeFormatMatches(current, 2) :
        (nativeFormatMatches(current, 0) || nativeFormatMatches(current, 1) || nativeFormatMatches(current, 2));
    if (current) CFRelease(current);
    return valid && currentValid;
}

int32_t wp_camera_is_registered(void) {
    UInt32 *devices = NULL, count = 0, matches = 0;
    CMIODeviceID device = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
        address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal), &devices, &count)) return 0;
    for (UInt32 i = 0; i < count; ++i) if (deviceUIDMatches(devices[i])) { device = devices[i]; ++matches; }
    free(devices);
    if (matches != 1) return 0;
    UInt32 *streams = NULL, streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeInput),
        &streams, &streamCount)) return 0;
    BOOL sourceValid = streamCount == 1 && nativeStreamMatches(streams[0], NO);
    free(streams);
    if (!sourceValid) return 0;
    streams = NULL; streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
        &streams, &streamCount)) return 0;
    BOOL sinkValid = streamCount == 1 && nativeStreamMatches(streams[0], YES);
    free(streams);
    return sinkValid ? 1 : 0;
}

int32_t wp_camera_open(WPCameraProducer **out) {
    if (!out) return WP_CAMERA_INVALID;
    *out = NULL;
    if (wp_camera_is_registered() != 1) return WP_CAMERA_NOT_INSTALLED;
    UInt32 *devices = NULL, count = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
                 address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal),
                 &devices, &count)) return WP_CAMERA_NOT_INSTALLED;
    CMIODeviceID device = 0;
    for (UInt32 i = 0; i < count; ++i) {
        if (deviceUIDMatches(devices[i])) { device = devices[i]; break; }
    }
    free(devices);
    if (!device) return WP_CAMERA_NOT_INSTALLED;
    UInt32 *streams = NULL, streamCount = 0;
    // A CMIO sink is an output stream from the producer application's view.
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
                 &streams, &streamCount)) return WP_CAMERA_NOT_INSTALLED;
    // This first extension publishes exactly one output stream. Fail closed
    // rather than accidentally writing to an unrelated/ambiguous stream.
    CMIOStreamID sink = streamCount == 1 ? streams[0] : 0;
    free(streams);
    if (!sink) return WP_CAMERA_NOT_INSTALLED;
    WPCameraProducer *p = calloc(1, sizeof(*p));
    if (!p) return WP_CAMERA_OS_ERROR;
    p->device = device; p->sink = sink; p->lastPTS = -1;
    p->width = WP_CAMERA_WIDTH; p->height = WP_CAMERA_HEIGHT;
    NSDictionary *attrs = @{
        (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (id)kCVPixelBufferWidthKey: @(WP_CAMERA_WIDTH),
        (id)kCVPixelBufferHeightKey: @(WP_CAMERA_HEIGHT),
        (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
    };
    OSStatus status = CVPixelBufferPoolCreate(kCFAllocatorDefault, NULL,
                                              (__bridge CFDictionaryRef)attrs, &p->pool);
    if (status == noErr) status = CMVideoFormatDescriptionCreate(kCFAllocatorDefault,
        kCVPixelFormatType_32BGRA, WP_CAMERA_WIDTH, WP_CAMERA_HEIGHT, NULL, &p->format);
    if (status == noErr) status = CMIOStreamCopyBufferQueue(sink, NULL, NULL, &p->queue);
    if (status == noErr && p->queue) status = CMIODeviceStartStream(device, sink);
    else if (status == noErr) status = -1;
    if (status != noErr) {
        if (p->queue) CFRelease(p->queue);
        if (p->format) CFRelease(p->format);
        if (p->pool) CFRelease(p->pool);
        free(p); return WP_CAMERA_OS_ERROR;
    }
    p->started = YES;
    *out = p; return WP_CAMERA_OK;
}

// Exact OBS public metadata observed by Root; never select by display name.
#define WP_OBS_DEVICE_UID "7626645E-4425-469E-9D8B-97E0FA59AC75"
#define WP_LIVELOOP_DEVICE_UID "5C0B4E2A-1F3D-4A6B-9C7E-1D2E3F4A5B6C"
#define WP_LIVELOOP_NAME_SELECTOR UINT32_C(0x6c6c636e)
#define WP_OBS_WIDTH 1920
#define WP_OBS_HEIGHT 1080
#define WP_OBS_PIXEL_FORMAT kCVPixelFormatType_32BGRA

static CFTypeRef copyProperty(CMIOObjectID object, CMIOObjectPropertySelector selector,
                              CFTypeID expectedType) {
    CMIOObjectPropertyAddress a = address(selector, kCMIOObjectPropertyScopeGlobal);
    CFTypeRef value = NULL;
    UInt32 used = 0;
    OSStatus status = CMIOObjectGetPropertyData(object, &a, 0, NULL, sizeof(value), &used, &value);
    if (status != noErr || used != sizeof(value) || !value || CFGetTypeID(value) != expectedType) {
        if (value) CFRelease(value);
        return NULL;
    }
    return value;
}

static BOOL obsFormatMatches(CMFormatDescriptionRef format) {
    if (!format || CFGetTypeID(format) != CMFormatDescriptionGetTypeID() ||
        CMFormatDescriptionGetMediaType(format) != kCMMediaType_Video) return NO;
    CMVideoDimensions dimensions = CMVideoFormatDescriptionGetDimensions(format);
    return CMFormatDescriptionGetMediaSubType(format) == WP_OBS_PIXEL_FORMAT &&
        dimensions.width == WP_OBS_WIDTH && dimensions.height == WP_OBS_HEIGHT;
}

static BOOL obsStreamMatches(CMIOStreamID stream, UInt32 expectedDirection) {
    UInt32 direction = UINT32_MAX, used = 0;
    CMIOObjectPropertyAddress a = address(kCMIOStreamPropertyDirection, kCMIOObjectPropertyScopeGlobal);
    if (CMIOObjectGetPropertyData(stream, &a, 0, NULL, sizeof(direction), &used, &direction) != noErr ||
        used != sizeof(direction) || direction != expectedDirection) return NO;
    CMFormatDescriptionRef current = (CMFormatDescriptionRef)copyProperty(stream,
        kCMIOStreamPropertyFormatDescription, CMFormatDescriptionGetTypeID());
    if (!current) return NO;
    BOOL exact = obsFormatMatches(current);
    CFRelease(current);
    if (!exact) return NO;
    CFArrayRef formats = (CFArrayRef)copyProperty(stream, kCMIOStreamPropertyFormatDescriptions,
                                                 CFArrayGetTypeID());
    if (!formats) return NO;
    exact = CFArrayGetCount(formats) == 1 &&
        obsFormatMatches((CMFormatDescriptionRef)CFArrayGetValueAtIndex(formats, 0));
    CFRelease(formats);
    if (!exact) return NO;
    return YES;
}

// Setup routing only: 0 means confirmed absence, 1 one exact UID;
// failed/ambiguous metadata is an error, never permission to install.
int32_t wp_liveloop_camera_is_present(void) {
    CMIOObjectPropertyAddress a = address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal);
    UInt32 size = 0, used = 0;
    if (CMIOObjectGetPropertyDataSize(kCMIOObjectSystemObject, &a, 0, NULL, &size) != noErr ||
        size > 65536 || size % sizeof(UInt32) != 0) return WP_CAMERA_OS_ERROR;
    if (size == 0) return 0;
    UInt32 *devices = calloc(1, size);
    if (!devices) return WP_CAMERA_OS_ERROR;
    if (CMIOObjectGetPropertyData(kCMIOObjectSystemObject, &a, 0, NULL, size, &used, devices) != noErr ||
        used != size) { free(devices); return WP_CAMERA_OS_ERROR; }
    UInt32 matches = 0;
    for (UInt32 i = 0; i < used / sizeof(UInt32); ++i) {
        CFStringRef uid = (CFStringRef)copyProperty(devices[i], kCMIODevicePropertyDeviceUID,
                                                  CFStringGetTypeID());
        if (!uid) { free(devices); return WP_CAMERA_OS_ERROR; }
        if (CFEqual(uid, CFSTR(WP_LIVELOOP_DEVICE_UID))) ++matches;
        CFRelease(uid);
    }
    free(devices);
    return matches <= 1 ? (int32_t)matches : WP_CAMERA_INVALID;
}

static BOOL providerSink(CFStringRef expectedUID, CMIODeviceID *outDevice, CMIOStreamID *outSink) {
    *outDevice = 0; *outSink = 0;
    UInt32 *devices = NULL, count = 0, matches = 0;
    CMIODeviceID device = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
                 address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal),
                 &devices, &count)) return NO;
    for (UInt32 i = 0; i < count; ++i) {
        CFStringRef uid = (CFStringRef)copyProperty(devices[i], kCMIODevicePropertyDeviceUID,
                                                  CFStringGetTypeID());
        if (uid && CFEqual(uid, expectedUID)) { device = devices[i]; ++matches; }
        if (uid) CFRelease(uid);
    }
    free(devices);
    if (matches != 1) return NO;
    UInt32 *sources = NULL, sourceCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeInput),
                 &sources, &sourceCount)) return NO;
    CMIOStreamID source = sourceCount == 1 ? sources[0] : 0;
    free(sources);
    if (!source || !obsStreamMatches(source, 1)) return NO;
    UInt32 *streams = NULL, streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
                 &streams, &streamCount)) return NO;
    CMIOStreamID sink = streamCount == 1 ? streams[0] : 0;
    free(streams);
    if (!sink) return NO;
    if (!obsStreamMatches(sink, 0)) return NO;
    if (CFEqual(expectedUID, CFSTR(WP_LIVELOOP_DEVICE_UID))) {
        CFStringRef sourceName = (CFStringRef)copyProperty(source, kCMIOObjectPropertyName, CFStringGetTypeID());
        CFStringRef sinkName = (CFStringRef)copyProperty(sink, kCMIOObjectPropertyName, CFStringGetTypeID());
        BOOL matches = sourceName && sinkName && CFEqual(sourceName, CFSTR("LiveLoop.Video.Source")) &&
            CFEqual(sinkName, CFSTR("LiveLoop.Video.Sink"));
        if (sourceName) CFRelease(sourceName);
        if (sinkName) CFRelease(sinkName);
        CMIOObjectPropertyAddress nameAddress = address(WP_LIVELOOP_NAME_SELECTOR, kCMIOObjectPropertyScopeGlobal);
        Boolean settable = false;
        if (!matches || CMIOObjectIsPropertySettable(device, &nameAddress, &settable) != noErr || !settable) return NO;
    }
    *outDevice = device; *outSink = sink;
    return YES;
}

int32_t wp_obs_camera_is_registered(void) {
    CMIODeviceID device;
    CMIOStreamID sink;
    return providerSink(CFSTR(WP_OBS_DEVICE_UID), &device, &sink) ? 1 : 0;
}

static BOOL liveLoopNameMatches(CMIODeviceID device) {
    CFStringRef name = (CFStringRef)copyProperty(device, kCMIOObjectPropertyName, CFStringGetTypeID());
    BOOL matches = name && CFEqual(name, CFSTR("Waypair Camera"));
    if (name) CFRelease(name);
    return matches;
}

int32_t wp_liveloop_camera_is_registered(void) {
    CMIODeviceID device;
    CMIOStreamID sink;
    return providerSink(CFSTR(WP_LIVELOOP_DEVICE_UID), &device, &sink) && liveLoopNameMatches(device) ? 1 : 0;
}

// Explicit idle preparation only. A successful write republishes asynchronously;
// callers must observe the new device/name before claiming a producer.
int32_t wp_liveloop_camera_prepare_name(void) {
    CMIODeviceID device;
    CMIOStreamID sink;
    if (!providerSink(CFSTR(WP_LIVELOOP_DEVICE_UID), &device, &sink)) return WP_CAMERA_NOT_INSTALLED;
    if (liveLoopNameMatches(device)) return WP_CAMERA_OK;
    UInt32 running = UINT32_MAX, used = 0;
    CMIOObjectPropertyAddress a = address(kCMIODevicePropertyDeviceIsRunningSomewhere, kCMIOObjectPropertyScopeGlobal);
    if (CMIOObjectGetPropertyData(device, &a, 0, NULL, sizeof(running), &used, &running) != noErr ||
        used != sizeof(running)) return WP_CAMERA_OS_ERROR;
    if (running != 0) return WP_CAMERA_BUSY;
    CMIODeviceID currentDevice;
    CMIOStreamID currentSink;
    if (!providerSink(CFSTR(WP_LIVELOOP_DEVICE_UID), &currentDevice, &currentSink) ||
        currentDevice != device || currentSink != sink) return WP_CAMERA_NOT_INSTALLED;
    a = address(WP_LIVELOOP_NAME_SELECTOR, kCMIOObjectPropertyScopeGlobal);
    CFStringRef name = CFSTR("Waypair Camera");
    OSStatus status = CMIOObjectSetPropertyData(device, &a, 0, NULL, sizeof(name), &name);
    return status == noErr ? WP_CAMERA_NAME_PENDING : WP_CAMERA_OS_ERROR;
}


// Static app-owned card only: no camera, screen, photo, or audio input.
static NSData *waypairIdleJPEG(void) {
    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc]
        initWithBitmapDataPlanes:NULL pixelsWide:1920 pixelsHigh:1080 bitsPerSample:8
        samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace
        bytesPerRow:0 bitsPerPixel:0];
    if (!bitmap) return nil;
    NSGraphicsContext *context = [NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap];
    if (!context) return nil;
    [NSGraphicsContext saveGraphicsState];
    [NSGraphicsContext setCurrentContext:context];
    [[NSColor colorWithCalibratedRed:0.07 green:0.09 blue:0.13 alpha:1] setFill];
    NSRectFill(NSMakeRect(0, 0, 1920, 1080));
    NSArray<NSString *> *lines = @[@"Waypair Camera", @"No phone video",
        @"Open Waypair Link to start or rearm; approve on your phone."];
    CGFloat sizes[] = {88, 48, 32};
    CGFloat positions[] = {600, 490, 370};
    for (NSUInteger i = 0; i < lines.count; ++i) {
        NSDictionary *attributes = @{
            NSFontAttributeName: [NSFont systemFontOfSize:sizes[i]],
            NSForegroundColorAttributeName: NSColor.whiteColor
        };
        NSSize size = [lines[i] sizeWithAttributes:attributes];
        [lines[i] drawAtPoint:NSMakePoint((1920 - size.width) / 2, positions[i])
              withAttributes:attributes];
    }
    [context flushGraphics];
    [NSGraphicsContext restoreGraphicsState];
    NSData *jpeg = [bitmap representationUsingType:NSBitmapImageFileTypeJPEG
        properties:@{NSImageCompressionFactor: @0.85}];
    if (!jpeg || jpeg.length == 0 || jpeg.length > 4000000) return nil;
    NSBitmapImageRep *decoded = [NSBitmapImageRep imageRepWithData:jpeg];
    return decoded && decoded.pixelsWide == 1920 && decoded.pixelsHigh == 1080 ? jpeg : nil;
}


#if defined(WP_CAMERA_IDLE_SCREEN_TEST)
// Source-only harness seam: performs no CMIO read/write and opens no producer.
int32_t wp_camera_test_idle_jpeg(void) {
    @autoreleasepool {
        NSData *jpeg = waypairIdleJPEG();
        NSBitmapImageRep *decoded = jpeg ? [NSBitmapImageRep imageRepWithData:jpeg] : nil;
        return jpeg.length > 0 && jpeg.length <= 4000000 && decoded &&
            decoded.pixelsWide == 1920 && decoded.pixelsHigh == 1080 ? 0 : -1;
    }
}
#endif

// Explicit metadata write only. The public property is write-only; no image readback proof.
int32_t wp_liveloop_camera_use_idle_screen(void) {
    @autoreleasepool {
        CMIODeviceID device = 0, currentDevice = 0;
        CMIOStreamID sink = 0, currentSink = 0;
        if (wp_liveloop_camera_is_present() != 1 ||
            !providerSink(CFSTR(WP_LIVELOOP_DEVICE_UID), &device, &sink)) return WP_CAMERA_NOT_INSTALLED;
        CMIOObjectPropertyAddress picture = address(UINT32_C(0x706c6368), kCMIOObjectPropertyScopeGlobal);
        Boolean settable = false;
        OSStatus status = CMIOObjectIsPropertySettable(device, &picture, &settable);
        if (status != noErr) return status;
        if (!settable) return WP_CAMERA_INVALID;
        NSData *jpeg = waypairIdleJPEG();
        if (!jpeg) return WP_CAMERA_INVALID;
        NSString *encoded = [jpeg base64EncodedStringWithOptions:0];
        if (wp_liveloop_camera_is_present() != 1 ||
            !providerSink(CFSTR(WP_LIVELOOP_DEVICE_UID), &currentDevice, &currentSink) ||
            currentDevice != device || currentSink != sink) return WP_CAMERA_NOT_INSTALLED;
        UInt32 running = UINT32_MAX, used = 0;
        CMIOObjectPropertyAddress a = address(kCMIODevicePropertyDeviceIsRunningSomewhere,
                                             kCMIOObjectPropertyScopeGlobal);
        status = CMIOObjectGetPropertyData(device, &a, 0, NULL, sizeof(running), &used, &running);
        if (status != noErr) return status;
        if (used != sizeof(running)) return WP_CAMERA_INVALID;
        if (running != 0) return WP_CAMERA_BUSY;
        CFStringRef value = (__bridge CFStringRef)encoded;
        return CMIOObjectSetPropertyData(device, &picture, 0, NULL, sizeof(value), &value);
    }
}

// CMIO's callback has no producer context and remains safe after handle close.
static void obsQueueAltered(CMIOStreamID stream, void *token, void *refCon) {
    (void)stream;
    (void)token;
    (void)refCon;
}

static int32_t providerOpen(CFStringRef expectedUID, WPCameraProducer **out) {
    if (!out) return WP_CAMERA_INVALID;
    *out = NULL;
    CMIODeviceID device;
    CMIOStreamID sink;
    if (!providerSink(expectedUID, &device, &sink)) return WP_CAMERA_NOT_INSTALLED;
    if (CFEqual(expectedUID, CFSTR(WP_LIVELOOP_DEVICE_UID)) && !liveLoopNameMatches(device))
        return WP_CAMERA_NAME_PENDING;
    WPCameraProducer *p = calloc(1, sizeof(*p));
    if (!p) return WP_CAMERA_OS_ERROR;
    p->device = device; p->sink = sink; p->lastPTS = -1;
    p->width = WP_OBS_WIDTH; p->height = WP_OBS_HEIGHT;
    p->providerUID = expectedUID;
    NSDictionary *attrs = @{
        (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (id)kCVPixelBufferWidthKey: @(p->width),
        (id)kCVPixelBufferHeightKey: @(p->height),
        (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
    };
    OSStatus status = CVPixelBufferPoolCreate(kCFAllocatorDefault, NULL,
        (__bridge CFDictionaryRef)attrs, &p->pool);
    if (status == noErr) status = CMVideoFormatDescriptionCreate(kCFAllocatorDefault,
        kCVPixelFormatType_32BGRA, p->width, p->height, NULL, &p->format);
    CMIODeviceID currentDevice = 0;
    CMIOStreamID currentSink = 0;
    if (status == noErr && (!providerSink(expectedUID, &currentDevice, &currentSink) ||
        currentDevice != device || currentSink != sink)) status = -1;
    if (status == noErr) status = CMIOStreamCopyBufferQueue(sink, obsQueueAltered, NULL, &p->queue);
    if (status == noErr && p->queue && CMSimpleQueueGetCapacity(p->queue) > 0)
        status = CMIODeviceStartStream(device, sink);
    else if (status == noErr) status = -1;
    if (status != noErr) {
        if (p->queue) CFRelease(p->queue);
        if (p->format) CFRelease(p->format);
        if (p->pool) CFRelease(p->pool);
        free(p); return WP_CAMERA_OS_ERROR;
    }
    p->started = YES;
    *out = p;
    return WP_CAMERA_OK;
}

int32_t wp_obs_camera_open(WPCameraProducer **out) {
    return providerOpen(CFSTR(WP_OBS_DEVICE_UID), out);
}

int32_t wp_liveloop_camera_open(WPCameraProducer **out) {
    return providerOpen(CFSTR(WP_LIVELOOP_DEVICE_UID), out);
}

// Re-discover the selected provider without reopening it. A replacement with
// the same UID but different live IDs does not inherit this producer's owner.
static BOOL producerTargetCurrent(WPCameraProducer *p) {
    if (!p->providerUID) return YES;
    CMIODeviceID device = 0;
    CMIOStreamID sink = 0;
    return providerSink(p->providerUID, &device, &sink) && device == p->device && sink == p->sink &&
        (!CFEqual(p->providerUID, CFSTR(WP_LIVELOOP_DEVICE_UID)) || liveLoopNameMatches(device));
}

static int32_t producerDropResult(WPCameraProducer *p) {
    return producerTargetCurrent(p) ? WP_CAMERA_DROPPED : WP_CAMERA_NOT_INSTALLED;
}

int32_t wp_camera_push_bgrx(WPCameraProducer *p, const uint8_t *bytes,
                           size_t length, uint32_t width, uint32_t height) {
    if (!p || !p->started || !bytes || !width || !height || width > 8192 || height > 8192 ||
        length != (size_t)width * height * 4) return WP_CAMERA_INVALID;
    if (!producerTargetCurrent(p)) return WP_CAMERA_NOT_INSTALLED;
    // Drop before allocating/converting when the extension has not consumed.
    // Keep at most two in-flight images; never accumulate seconds of latency.
    if (CMSimpleQueueGetCount(p->queue) >= MIN(2, CMSimpleQueueGetCapacity(p->queue)))
        return WP_CAMERA_DROPPED;
    CMTime began = CMClockGetTime(CMClockGetHostTimeClock());
    CVPixelBufferRef pixel = NULL;
    NSDictionary *limit = @{(id)kCVPixelBufferPoolAllocationThresholdKey: @4};
    CVReturn cv = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault,
        p->pool, (__bridge CFDictionaryRef)limit, &pixel);
    if (cv == kCVReturnWouldExceedAllocationThreshold) return producerDropResult(p);
    if (cv != kCVReturnSuccess || !pixel) {
        if (pixel) CFRelease(pixel);
        return WP_CAMERA_OS_ERROR;
    }
    if (CVPixelBufferLockBaseAddress(pixel, 0) != kCVReturnSuccess) {
        CFRelease(pixel); return WP_CAMERA_OS_ERROR;
    }
    size_t pitch = CVPixelBufferGetBytesPerRow(pixel);
    uint8_t *base = CVPixelBufferGetBaseAddress(pixel);
    if (!base || pitch < (size_t)p->width * 4) {
        CVPixelBufferUnlockBaseAddress(pixel, 0); CFRelease(pixel); return WP_CAMERA_OS_ERROR;
    }
    memset(base, 0, pitch * p->height);
    // Preserve portrait and landscape aspect ratio; all clients see one stable format.
    double scale = fmin((double)p->width / width, (double)p->height / height);
    size_t w = MAX(1, (size_t)(width * scale)), h = MAX(1, (size_t)(height * scale));
    size_t x = (p->width - w) / 2, y = (p->height - h) / 2;
    vImage_Buffer source = {(void *)bytes, height, width, (size_t)width * 4};
    vImage_Buffer target = {base + y * pitch + x * 4, h, w, pitch};
    vImage_Error imageStatus = vImageScale_ARGB8888(&source, &target, NULL, kvImageHighQualityResampling);
    // BGRX and BGRA share channel layout, but X is not a valid alpha value.
    for (size_t row = 0; row < p->height; ++row)
        for (size_t col = 0; col < p->width; ++col) base[row * pitch + col * 4 + 3] = 255;
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    if (imageStatus != kvImageNoError) { CFRelease(pixel); return WP_CAMERA_OS_ERROR; }
    CMTime host = CMClockGetTime(CMClockGetHostTimeClock());
    if (!CMTIME_IS_NUMERIC(began) || !CMTIME_IS_NUMERIC(host) ||
        CMTimeCompare(CMTimeSubtract(host, began), CMTimeMake(WP_CAMERA_MAX_AGE_NS, 1000000000)) > 0) {
        CFRelease(pixel); return producerDropResult(p);
    }
    int64_t ns = CMTimeConvertScale(host, 1000000000, kCMTimeRoundingMethod_Default).value;
    if (ns <= p->lastPTS) ns = p->lastPTS + 1;
    p->lastPTS = ns;
    CMSampleTimingInfo timing = {CMTimeMake(1, WP_CAMERA_FPS), CMTimeMake(ns, 1000000000), kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    OSStatus status = CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, pixel,
        p->format, &timing, &sample);
    CFRelease(pixel);
    if (status != noErr || !sample) {
        if (sample) CFRelease(sample);
        return WP_CAMERA_OS_ERROR;
    }
    // The CMIO sink consumes the +1 sample reference on successful enqueue.
    // A CMSimpleQueue itself does not retain elements.
    if (!producerTargetCurrent(p)) { CFRelease(sample); return WP_CAMERA_NOT_INSTALLED; }
    status = CMSimpleQueueEnqueue(p->queue, sample);
    if (status != noErr) CFRelease(sample);
    return status == noErr ? WP_CAMERA_OK : producerDropResult(p);
}

void wp_camera_close(WPCameraProducer *p) {
    if (!p) return;
    p->started = NO;
    CMIODeviceStopStream(p->device, p->sink);
    // Do not dequeue/reset: CMIO is the queue's sole consumer.
    CFRelease(p->queue); CFRelease(p->format); CFRelease(p->pool);
    free(p);
}

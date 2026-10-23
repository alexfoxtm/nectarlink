// SPDX-License-Identifier: GPL-3.0-or-later
// Compiles into the existing Rust/Qt application, not into a replacement app.
#import <Foundation/Foundation.h>
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
    BOOL obs;
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
    free(streams);
    if (streamCount != 1) return 0;
    streams = NULL; streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
        &streams, &streamCount)) return 0;
    free(streams);
    return streamCount == 1 ? 1 : 0;
}

int32_t wp_camera_open(WPCameraProducer **out) {
    if (!out) return WP_CAMERA_INVALID;
    *out = NULL;
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

static BOOL obsSink(CMIODeviceID *outDevice, CMIOStreamID *outSink) {
    *outDevice = 0; *outSink = 0;
    UInt32 *devices = NULL, count = 0, matches = 0;
    CMIODeviceID device = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
                 address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal),
                 &devices, &count)) return NO;
    for (UInt32 i = 0; i < count; ++i) {
        CFStringRef uid = (CFStringRef)copyProperty(devices[i], kCMIODevicePropertyDeviceUID,
                                                  CFStringGetTypeID());
        if (uid && CFEqual(uid, CFSTR(WP_OBS_DEVICE_UID))) { device = devices[i]; ++matches; }
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
    *outDevice = device; *outSink = sink;
    return YES;
}

int32_t wp_obs_camera_is_registered(void) {
    CMIODeviceID device;
    CMIOStreamID sink;
    return obsSink(&device, &sink) ? 1 : 0;
}

// CMIO's callback has no producer context and remains safe after handle close.
static void obsQueueAltered(CMIOStreamID stream, void *token, void *refCon) {
    (void)stream;
    (void)token;
    (void)refCon;
}

int32_t wp_obs_camera_open(WPCameraProducer **out) {
    if (!out) return WP_CAMERA_INVALID;
    *out = NULL;
    CMIODeviceID device;
    CMIOStreamID sink;
    if (!obsSink(&device, &sink)) return WP_CAMERA_NOT_INSTALLED;
    WPCameraProducer *p = calloc(1, sizeof(*p));
    if (!p) return WP_CAMERA_OS_ERROR;
    p->device = device; p->sink = sink; p->lastPTS = -1;
    p->width = WP_OBS_WIDTH; p->height = WP_OBS_HEIGHT;
    p->obs = YES;
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
    if (status == noErr && (!obsSink(&currentDevice, &currentSink) ||
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

// Re-discover the selected provider without reopening it. A replacement with
// the same UID but different live IDs does not inherit this producer's owner.
static BOOL producerTargetCurrent(WPCameraProducer *p) {
    if (!p->obs) return YES;
    CMIODeviceID device = 0;
    CMIOStreamID sink = 0;
    return obsSink(&device, &sink) && device == p->device && sink == p->sink;
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

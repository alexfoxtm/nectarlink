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
    *out = p; return WP_CAMERA_OK;
}

int32_t wp_camera_push_bgrx(WPCameraProducer *p, const uint8_t *bytes,
                           size_t length, uint32_t width, uint32_t height) {
    if (!p || !bytes || !width || !height || width > 8192 || height > 8192 ||
        length != (size_t)width * height * 4) return WP_CAMERA_INVALID;
    // Drop before allocating/converting when the extension has not consumed.
    // Keep at most two in-flight images; never accumulate seconds of latency.
    if (CMSimpleQueueGetCount(p->queue) >= 2) return WP_CAMERA_DROPPED;
    CVPixelBufferRef pixel = NULL;
    NSDictionary *limit = @{(id)kCVPixelBufferPoolAllocationThresholdKey: @4};
    CVReturn cv = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault,
        p->pool, (__bridge CFDictionaryRef)limit, &pixel);
    if (cv == kCVReturnWouldExceedAllocationThreshold) return WP_CAMERA_DROPPED;
    if (cv != kCVReturnSuccess || !pixel) return WP_CAMERA_OS_ERROR;
    if (CVPixelBufferLockBaseAddress(pixel, 0) != kCVReturnSuccess) {
        CFRelease(pixel); return WP_CAMERA_OS_ERROR;
    }
    size_t pitch = CVPixelBufferGetBytesPerRow(pixel);
    uint8_t *base = CVPixelBufferGetBaseAddress(pixel);
    memset(base, 0, pitch * WP_CAMERA_HEIGHT);
    // Preserve portrait and landscape aspect ratio; all clients see one stable format.
    double scale = fmin((double)WP_CAMERA_WIDTH / width, (double)WP_CAMERA_HEIGHT / height);
    size_t w = MAX(1, (size_t)(width * scale)), h = MAX(1, (size_t)(height * scale));
    size_t x = (WP_CAMERA_WIDTH - w) / 2, y = (WP_CAMERA_HEIGHT - h) / 2;
    vImage_Buffer source = {(void *)bytes, height, width, (size_t)width * 4};
    vImage_Buffer target = {base + y * pitch + x * 4, h, w, pitch};
    vImage_Error imageStatus = vImageScale_ARGB8888(&source, &target, NULL, kvImageHighQualityResampling);
    // BGRX and BGRA share channel layout, but X is not a valid alpha value.
    for (size_t row = 0; row < WP_CAMERA_HEIGHT; ++row)
        for (size_t col = 0; col < WP_CAMERA_WIDTH; ++col) base[row * pitch + col * 4 + 3] = 255;
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    if (imageStatus != kvImageNoError) { CFRelease(pixel); return WP_CAMERA_OS_ERROR; }
    CMTime host = CMClockGetTime(CMClockGetHostTimeClock());
    int64_t ns = CMTimeConvertScale(host, 1000000000, kCMTimeRoundingMethod_Default).value;
    if (ns <= p->lastPTS) ns = p->lastPTS + 1;
    p->lastPTS = ns;
    CMSampleTimingInfo timing = {CMTimeMake(1, WP_CAMERA_FPS), CMTimeMake(ns, 1000000000), kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    OSStatus status = CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, pixel,
        p->format, &timing, &sample);
    CFRelease(pixel);
    if (status != noErr || !sample) return WP_CAMERA_OS_ERROR;
    // The CMIO sink consumes the +1 sample reference on successful enqueue.
    // A CMSimpleQueue itself does not retain elements.
    status = CMSimpleQueueEnqueue(p->queue, sample);
    if (status != noErr) CFRelease(sample);
    return status == noErr ? WP_CAMERA_OK : WP_CAMERA_DROPPED;
}

void wp_camera_close(WPCameraProducer *p) {
    if (!p) return;
    CMIODeviceStopStream(p->device, p->sink);
    // Do not dequeue/reset: CMIO is the queue's sole consumer.
    CFRelease(p->queue); CFRelease(p->format); CFRelease(p->pool); free(p);
}

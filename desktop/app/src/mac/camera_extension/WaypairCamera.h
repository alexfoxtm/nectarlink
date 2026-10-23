// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <stddef.h>
#include <stdint.h>

#define WP_CAMERA_DEVICE_UUID "B1E51A52-92BE-4A8D-A4CC-146CB79C9953"
#define WP_CAMERA_SOURCE_UUID "A943EF3A-7150-47C4-8DBA-5D6874D68069"
#define WP_CAMERA_SINK_UUID "E662A82D-F228-4392-BF51-FBB6091BCBE1"
#define WP_CAMERA_WIDTH 1280
#define WP_CAMERA_HEIGHT 720
#define WP_CAMERA_FPS 30
#define WP_CAMERA_MAX_AGE_NS UINT64_C(250000000)

#ifdef __cplusplus
extern "C" {
#endif
typedef struct WPCameraProducer WPCameraProducer;
enum { WP_CAMERA_OK = 0, WP_CAMERA_DROPPED = 1, WP_CAMERA_INVALID = -1,
       WP_CAMERA_NOT_INSTALLED = -2, WP_CAMERA_OS_ERROR = -3 };

// Open only after a local Start/phone-accepted session has an exclusive
// (device ID, stream generation) owner. No phone/permission/activation side effects.
// A published device UID alone is not proof of approved installation/signing.
int32_t wp_camera_is_registered(void);
int32_t wp_camera_open(WPCameraProducer **out);
// One serialized calling lane per handle. Tight BGRX comes directly from
// webcam.rs's existing to_bgrx + optional mirror step. Copies before returning.
// Letterboxes/scales to the extension's single 1280x720 BGRA format.
int32_t wp_camera_push_bgrx(WPCameraProducer *producer, const uint8_t *bytes,
                          size_t length, uint32_t width, uint32_t height);
// Stop the sink before relinquishing the capture owner. Never keep a producer
// alive across an owner/peer change. Calling close(NULL) is harmless.
void wp_camera_close(WPCameraProducer *producer);
#ifdef __cplusplus
}
#endif

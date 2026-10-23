// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
enum { WP_CAMERA_ACTIVATION_SUBMITTED = 0, WP_CAMERA_ACTIVATION_NEEDS_APPROVAL = 1,
       WP_CAMERA_ACTIVATION_COMPLETED = 2, WP_CAMERA_ACTIVATION_REBOOT = 3,
       WP_CAMERA_ACTIVATION_FAILED = 4, WP_CAMERA_ACTIVATION_BUSY = 5 };
typedef void (*WPCameraActivationCallback)(void *context, int32_t status, int64_t os_error);
// Explicit UI action only. Callback runs on main queue. Context must remain
// valid until terminal COMPLETED/REBOOT/FAILED/BUSY. Submitted/pending are not Ready.
// COMPLETED still requires a published-device readiness probe before capability advertisement.
void wp_camera_request_activation(int32_t enable, WPCameraActivationCallback callback, void *context);
#ifdef __cplusplus
}
#endif

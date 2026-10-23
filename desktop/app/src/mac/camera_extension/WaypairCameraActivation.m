// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import <SystemExtensions/SystemExtensions.h>
#include "WaypairCameraActivation.h"

static NSString *const extensionID = @"com.waypair.mac.CameraExtension";
@interface WPActivation : NSObject <OSSystemExtensionRequestDelegate>
@property(nonatomic, strong) OSSystemExtensionRequest *request;
@property(nonatomic) WPCameraActivationCallback callback;
@property(nonatomic) void *context;
@end
@implementation WPActivation
- (void)requestNeedsUserApproval:(OSSystemExtensionRequest *)request {
    if (request == self.request && self.callback)
        self.callback(self.context, WP_CAMERA_ACTIVATION_NEEDS_APPROVAL, 0);
}
- (OSSystemExtensionReplacementAction)request:(OSSystemExtensionRequest *)request
                actionForReplacingExtension:(OSSystemExtensionProperties *)existing
                              withExtension:(OSSystemExtensionProperties *)replacement {
    if (request != self.request || ![existing.bundleIdentifier isEqual:extensionID] ||
        ![replacement.bundleIdentifier isEqual:extensionID]) return OSSystemExtensionReplacementActionCancel;
    return [replacement.bundleVersion compare:existing.bundleVersion options:NSNumericSearch] != NSOrderedAscending
        ? OSSystemExtensionReplacementActionReplace : OSSystemExtensionReplacementActionCancel;
}
- (void)request:(OSSystemExtensionRequest *)request didFinishWithResult:(OSSystemExtensionRequestResult)result {
    if (request != self.request) return;
    WPCameraActivationCallback callback = self.callback; void *context = self.context;
    self.request = nil; self.callback = NULL; self.context = NULL;
    if (callback) callback(context, result == OSSystemExtensionRequestCompleted
        ? WP_CAMERA_ACTIVATION_COMPLETED : WP_CAMERA_ACTIVATION_REBOOT, 0);
}
- (void)request:(OSSystemExtensionRequest *)request didFailWithError:(NSError *)error {
    if (request != self.request) return;
    WPCameraActivationCallback callback = self.callback; void *context = self.context;
    self.request = nil; self.callback = NULL; self.context = NULL;
    if (callback) callback(context, WP_CAMERA_ACTIVATION_FAILED, error.code);
}
@end

void wp_camera_request_activation(int32_t enable, WPCameraActivationCallback callback, void *context) {
    dispatch_async(dispatch_get_main_queue(), ^{
        static WPActivation *activation;
        if (!activation) activation = [WPActivation new];
        if (activation.request) {
            if (callback) callback(context, WP_CAMERA_ACTIVATION_BUSY, 0);
            return;
        }
        activation.callback = callback; activation.context = context;
        activation.request = enable
            ? [OSSystemExtensionRequest activationRequestForExtension:extensionID queue:dispatch_get_main_queue()]
            : [OSSystemExtensionRequest deactivationRequestForExtension:extensionID queue:dispatch_get_main_queue()];
        activation.request.delegate = activation;
        if (callback) callback(context, WP_CAMERA_ACTIVATION_SUBMITTED, 0);
        [[OSSystemExtensionManager sharedManager] submitRequest:activation.request];
    });
}

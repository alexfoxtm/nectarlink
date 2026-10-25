// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import <CoreFoundation/CoreFoundation.h>
#include <stdbool.h>
#include <pthread.h>

// LiveLoop publishes these for first/last SOURCE consumer, independently of its producer sink.
static CFStringRef const activeName = CFSTR("com.adrbn.LiveLoop.consumerActive");
static CFStringRef const inactiveName = CFSTR("com.adrbn.LiveLoop.consumerInactive");
static pthread_mutex_t observerLock = PTHREAD_MUTEX_INITIALIZER;
static CFStringRef const nativeActiveName = CFSTR("com.waypair.camera.consumerActive");
static CFStringRef const nativeInactiveName = CFSTR("com.waypair.camera.consumerInactive");
static void (*edgeCallback)(bool, bool) = NULL;
static void demandChanged(CFNotificationCenterRef center, void *observer,
                          CFStringRef name, const void *object, CFDictionaryRef userInfo) {
    (void)center; (void)observer; (void)object; (void)userInfo;
    pthread_mutex_lock(&observerLock);
    if (edgeCallback) edgeCallback(CFEqual(name, nativeActiveName) || CFEqual(name, nativeInactiveName),
        CFEqual(name, activeName) || CFEqual(name, nativeActiveName));
    pthread_mutex_unlock(&observerLock);
}
void wp_camera_demand_start(void (*callback)(bool, bool)) {
    pthread_mutex_lock(&observerLock);
    if (!edgeCallback) {
        edgeCallback = callback;
        CFNotificationCenterRef center = CFNotificationCenterGetDarwinNotifyCenter();
        CFNotificationCenterAddObserver(center, &edgeCallback, demandChanged, activeName, NULL,
                                        CFNotificationSuspensionBehaviorDeliverImmediately);
        CFNotificationCenterAddObserver(center, &edgeCallback, demandChanged, inactiveName, NULL,
                                        CFNotificationSuspensionBehaviorDeliverImmediately);
        CFNotificationCenterAddObserver(center, &edgeCallback, demandChanged, nativeActiveName, NULL,
                                        CFNotificationSuspensionBehaviorDeliverImmediately);
        CFNotificationCenterAddObserver(center, &edgeCallback, demandChanged, nativeInactiveName, NULL,
                                        CFNotificationSuspensionBehaviorDeliverImmediately);
    }
    pthread_mutex_unlock(&observerLock);
}
void wp_camera_demand_stop(void) {
    pthread_mutex_lock(&observerLock);
    edgeCallback = NULL;
    CFNotificationCenterRemoveEveryObserver(CFNotificationCenterGetDarwinNotifyCenter(), &edgeCallback);
    pthread_mutex_unlock(&observerLock);
}

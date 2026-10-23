// SPDX-License-Identifier: GPL-3.0-or-later
#import <Foundation/Foundation.h>
#import <CoreWLAN/CoreWLAN.h>
#import <CoreLocation/CoreLocation.h>
#import <Security/Security.h>
#include <stdlib.h>
#include <stdbool.h>
#include <dispatch/dispatch.h>
#include <string.h>

// No subprocess, Wi-Fi power change, configuration commit, password logging or
// automatic permission request. The caller invokes association after a user tap.
static BOOL locationAllowed(void) {
    CLAuthorizationStatus status = [CLLocationManager new].authorizationStatus;
    return CLLocationManager.locationServicesEnabled &&
        status == kCLAuthorizationStatusAuthorizedAlways;
}

static NSString *errorText(NSError *error) {
    // NSError userInfo/description may contain the SSID or password.
    if ([error.domain isEqualToString:NSOSStatusErrorDomain] &&
        (error.code == errSecAuthFailed || error.code == errSecInteractionNotAllowed))
        return @"Allow Keychain access for this saved Wi-Fi network, then try again.";
    return @"macOS could not join the saved hotspot. Check Wi-Fi, Keychain access and the hotspot password in System Settings.";
}

static char *copyString(NSString *value) {
    return value ? strdup(value.UTF8String) : NULL;
}

char *waypair_wifi_permission_error(void) {
    @autoreleasepool {
        if ([CLLocationManager new].authorizationStatus == kCLAuthorizationStatusNotDetermined)
            return copyString(@"Choose Allow hotspot discovery in Join phone hotspot and allow the macOS permission prompt, then retry.");
        if (!locationAllowed()) return copyString(@"Allow Waypair Link in System Settings > Privacy & Security > Location Services to find the phone hotspot. You can also join it manually in Wi-Fi settings.");
        if (!CWWiFiClient.sharedWiFiClient.interface) return copyString(@"No Wi-Fi interface is available on this Mac.");
        if (!CWWiFiClient.sharedWiFiClient.interface.powerOn) return copyString(@"Turn on Wi-Fi in System Settings, then try again.");
        return NULL;
    }
}

char *waypair_wifi_request_permission(void) {
    @autoreleasepool {
        if (![NSThread isMainThread]) return copyString(@"Open hotspot setup from the main application window.");
        NSString *usage = [NSBundle.mainBundle objectForInfoDictionaryKey:@"NSLocationUsageDescription"];
        if (![usage isKindOfClass:NSString.class] || !usage.length)
            return copyString(@"This app bundle is missing its Wi-Fi discovery permission description. Install the packaged Waypair Link app.");
        static CLLocationManager *manager;
        static dispatch_once_t once;
        dispatch_once(&once, ^{ manager = [CLLocationManager new]; });
        if (manager.authorizationStatus == kCLAuthorizationStatusNotDetermined) [manager requestWhenInUseAuthorization];
        else if (!locationAllowed()) return copyString(@"Allow Waypair Link in System Settings > Privacy & Security > Location Services, then reopen hotspot setup.");
        return NULL;
    }
}

char *waypair_wifi_saved_networks(void) {
    @autoreleasepool {
        if (!locationAllowed()) return copyString(@"[]");
        NSMutableArray<NSString *> *names = [NSMutableArray new];
        for (CWNetworkProfile *profile in CWWiFiClient.sharedWiFiClient.interface.configuration.networkProfiles) {
            if (profile.ssid.length && ![names containsObject:profile.ssid]) [names addObject:profile.ssid];
        }
        [names sortUsingSelector:@selector(localizedCaseInsensitiveCompare:)];
        NSData *json = [NSJSONSerialization dataWithJSONObject:names options:0 error:nil];
        return json ? copyString([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding]) : copyString(@"[]");
    }
}

// 0: absent, 1: in range, 2: connected, 3: permission/interface/scan error.
int waypair_wifi_reach(const char *name) {
    @autoreleasepool {
        if (!name || !locationAllowed()) return 3;
        NSString *ssid = [NSString stringWithUTF8String:name];
        CWInterface *interface = CWWiFiClient.sharedWiFiClient.interface;
        if (!ssid || !interface || !interface.powerOn) return 3;
        NSData *data = [ssid dataUsingEncoding:NSUTF8StringEncoding];
        if ([interface.ssidData isEqualToData:data]) return 2;
        NSError *error = nil;
        NSSet<CWNetwork *> *networks = [interface scanForNetworksWithSSID:data error:&error];
        if (error || !networks) return 3;
        for (CWNetwork *network in networks) if ([network.ssidData isEqualToData:data]) return 1;
        return 0;
    }
}

// Returns NULL on association success, otherwise a non-sensitive message.
char *waypair_wifi_connect(const char *name, bool (*current)(void *), void *owner) {
    @autoreleasepool {
        if (!current || !current(owner)) return copyString(@"Hotspot join cancelled.");
        char *permission = waypair_wifi_permission_error();
        if (permission) return permission;
        NSString *ssid = name ? [NSString stringWithUTF8String:name] : nil;
        if (!ssid.length) return copyString(@"Choose the phone's saved hotspot in Settings > Devices first.");
        CWInterface *interface = CWWiFiClient.sharedWiFiClient.interface;
        NSData *data = [ssid dataUsingEncoding:NSUTF8StringEncoding];
        if ([interface.ssidData isEqualToData:data]) return NULL;
        NSError *error = nil;
        NSSet<CWNetwork *> *networks = [interface scanForNetworksWithSSID:data error:&error];
        if (!networks || error) return copyString(@"macOS could not scan for the hotspot. Check Location Services and Wi-Fi in System Settings.");
        CWNetwork *chosen = nil;
        for (CWNetwork *network in networks) if ([network.ssidData isEqualToData:data]) { chosen = network; break; }
        if (!chosen) return copyString(@"The phone hotspot is not in range yet.");
        // A protected, previously saved profile is required. Never silently
        // downgrade a remembered phone to an open AP advertising the same SSID.
        if (![chosen supportsSecurity:kCWSecurityWPA2Personal] &&
            ![chosen supportsSecurity:kCWSecurityWPA3Personal] &&
            ![chosen supportsSecurity:kCWSecurityWPA3Transition])
            return copyString(@"Use a WPA2 or WPA3 password-protected phone hotspot.");
        NSString *password = nil;
        OSStatus status = CWKeychainFindWiFiPassword(kCWKeychainDomainUser, data, &password);
        if (status == errSecItemNotFound)
            status = CWKeychainFindWiFiPassword(kCWKeychainDomainSystem, data, &password);
        if (status != errSecSuccess || !password.length)
            return copyString(@"Join the phone hotspot once in macOS Wi-Fi settings and allow Keychain access, then choose the saved network in Waypair Link.");
        // A Keychain prompt/scan can outlive the remote owner. Recheck at the
        // OS-action boundary. CoreWLAN cannot cancel an already submitted join.
        if (!current(owner)) return copyString(@"Hotspot join cancelled.");
        BOOL joined = [interface associateToNetwork:chosen password:password error:&error];
        password = nil;
        return joined && !error ? NULL : copyString(errorText(error));
    }
}

// The per-phone SSID is private Keychain data; no hotspot.json on macOS.
static NSDictionary *profileQuery(const char *peer) {
    NSString *account = peer ? [NSString stringWithUTF8String:peer] : nil;
    if (!account.length) return nil;
    return @{(__bridge id)kSecClass:(__bridge id)kSecClassGenericPassword,
             (__bridge id)kSecAttrService:@"com.waypair.link.phone-hotspot",
             (__bridge id)kSecAttrAccount:account,
             (__bridge id)kSecAttrSynchronizable:@NO};
}

char *waypair_wifi_profile(const char *peer) {
    @autoreleasepool {
        NSDictionary *base = profileQuery(peer);
        if (!base) return NULL;
        NSMutableDictionary *query = [base mutableCopy];
        query[(__bridge id)kSecReturnData] = @YES;
        query[(__bridge id)kSecMatchLimit] = (__bridge id)kSecMatchLimitOne;
        CFTypeRef result = NULL;
        if (SecItemCopyMatching((__bridge CFDictionaryRef)query, &result) != errSecSuccess) return NULL;
        NSData *data = CFBridgingRelease(result);
        return copyString([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding]);
    }
}

int waypair_wifi_set_profile(const char *peer, const char *name) {
    @autoreleasepool {
        NSDictionary *base = profileQuery(peer);
        if (!base) return errSecParam;
        if (!name) {
            OSStatus status = SecItemDelete((__bridge CFDictionaryRef)base);
            return status == errSecItemNotFound ? errSecSuccess : status;
        }
        NSString *ssid = [NSString stringWithUTF8String:name];
        NSData *data = [ssid dataUsingEncoding:NSUTF8StringEncoding];
        if (!data.length || data.length > 32) return errSecParam;
        NSDictionary *update = @{(__bridge id)kSecValueData:data};
        OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)base, (__bridge CFDictionaryRef)update);
        if (status == errSecItemNotFound) {
            NSMutableDictionary *item = [base mutableCopy];
            [item addEntriesFromDictionary:update];
            item[(__bridge id)kSecAttrAccessible] = (__bridge id)kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly;
            status = SecItemAdd((__bridge CFDictionaryRef)item, NULL);
        }
        return status;
    }
}

void waypair_wifi_free(char *value) { free(value); }

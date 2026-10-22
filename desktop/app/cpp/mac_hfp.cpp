// SPDX-License-Identifier: GPL-3.0-or-later
#include "mac_hfp.h"

#include <cstring>
#include <optional>
#include <string>

extern "C" {
typedef int (*WPHfpLeaseIsCurrent)(void *context, const char *bindingJSON);
typedef void (*WPHfpEvent)(void *context, const char *snapshotJSON);
uint64_t WPHfpStart(const char *bindingJSON, WPHfpLeaseIsCurrent lease, WPHfpEvent event, void *context);
int WPHfpHandback(uint64_t handle);
void WPHfpFreeJSON(char *json);
int WPHfpRelease(uint64_t handle);

char *WPHfpSupportCopyPrerequisitesJSON(void);
char *WPHfpSupportCopyPairedGatewaysJSON(void);
char *WPHfpSupportCopyPairedPhoneCandidatesJSON(void);
typedef void (*WPSDPRefreshEvent)(uint64_t request, const char *resultJSON);
int WPHfpSupportRefreshPhoneSDP(const char *address, uint64_t request, WPSDPRefreshEvent event);
char *WPHfpSupportCopyDefaultEndpointsJSON(void);
typedef void (*WPHfpSupportChanged)(void);
int WPHfpSupportRequestBluetooth(WPHfpSupportChanged changed);
int WPHfpSupportRequestMicrophone(WPHfpSupportChanged changed);
void WPHfpSupportPumpMainRunLoop(double seconds);
}

namespace {

using LeaseFn = rust::Fn<bool(uint64_t, rust::Slice<const uint8_t>)>;
using EventFn = rust::Fn<void(uint64_t, rust::Slice<const uint8_t>)>;

// The provider allows one session at a time and keeps its callbacks until
// release. The Rust functions behind these are plain `fn` items, so the
// latest pair is valid for every retained handle as well.
std::optional<LeaseFn> lease_fn;
std::optional<EventFn> event_fn;

// Both requests use the same plain Rust fn item, never a closure/context.
// Retained for late native notifications; reads and writes are Main-only.
std::optional<rust::Fn<void()>> prerequisites_changed_fn;

// The SDP check's plain Rust fn item, retained for the process lifetime like
// the prerequisites callback: an outstanding OS query may complete long after
// its request was logically cancelled. Main-only, like everything it calls.
std::optional<EventFn> sdp_event_fn;

void prerequisites_changed_trampoline()
{
    if (prerequisites_changed_fn)
        (*prerequisites_changed_fn)();
}

rust::Slice<const uint8_t> bytes(const char *text)
{
    return {reinterpret_cast<const uint8_t *>(text), std::strlen(text)};
}

uint64_t session_of(void *context)
{
    return static_cast<uint64_t>(reinterpret_cast<std::uintptr_t>(context));
}

int lease_trampoline(void *context, const char *binding_json)
{
    if (!lease_fn || !binding_json)
        return 0;
    return (*lease_fn)(session_of(context), bytes(binding_json)) ? 1 : 0;
}

void event_trampoline(void *context, const char *snapshot_json)
{
    if (event_fn && snapshot_json)
        (*event_fn)(session_of(context), bytes(snapshot_json));
}

void sdp_event_trampoline(uint64_t request, const char *result_json)
{
    if (sdp_event_fn && result_json)
        (*sdp_event_fn)(request, bytes(result_json));
}

rust::String take_json(char *json, bool &ok)
{
    ok = json != nullptr;
    if (!json)
        return {};
    rust::String text = rust::String::lossy(json);
    WPHfpFreeJSON(json);
    return text;
}

} // namespace

uint64_t mac_hfp_start(rust::Str binding_json, uint64_t session, LeaseFn lease, EventFn event)
{
    if (session == 0)
        return 0;
    const std::string json(binding_json.data(), binding_json.size());
    if (json.find('\0') != std::string::npos)
        return 0;
    lease_fn = lease;
    event_fn = event;
    void *context = reinterpret_cast<void *>(static_cast<std::uintptr_t>(session));
    return WPHfpStart(json.c_str(), lease_trampoline, event_trampoline, context);
}

bool mac_hfp_handback(uint64_t handle)
{
    return handle != 0 && WPHfpHandback(handle) == 1;
}

bool mac_hfp_release(uint64_t handle)
{
    return handle != 0 && WPHfpRelease(handle) == 1;
}

rust::String mac_hfp_prerequisites()
{
    bool ok = false;
    return take_json(WPHfpSupportCopyPrerequisitesJSON(), ok);
}

rust::String mac_hfp_paired_gateways(bool &ok)
{
    return take_json(WPHfpSupportCopyPairedGatewaysJSON(), ok);
}

rust::String mac_hfp_paired_phone_candidates(bool &ok)
{
    return take_json(WPHfpSupportCopyPairedPhoneCandidatesJSON(), ok);
}

int32_t mac_hfp_refresh_phone_sdp(rust::Str address, uint64_t request, EventFn event)
{
    if (request == 0 || address.empty())
        return 0;
    const std::string value(address.data(), address.size());
    if (value.find('\0') != std::string::npos)
        return 0;
    sdp_event_fn = event;
    return WPHfpSupportRefreshPhoneSDP(value.c_str(), request, sdp_event_trampoline);
}

rust::String mac_hfp_default_endpoints(bool &ok)
{
    return take_json(WPHfpSupportCopyDefaultEndpointsJSON(), ok);
}

bool mac_hfp_request_bluetooth(rust::Fn<void()> changed)
{
    prerequisites_changed_fn = changed;
    return WPHfpSupportRequestBluetooth(prerequisites_changed_trampoline) == 1;
}

bool mac_hfp_request_microphone(rust::Fn<void()> changed)
{
    prerequisites_changed_fn = changed;
    return WPHfpSupportRequestMicrophone(prerequisites_changed_trampoline) == 1;
}

void mac_hfp_pump_main_run_loop(double seconds)
{
    WPHfpSupportPumpMainRunLoop(seconds);
}

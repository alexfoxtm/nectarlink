// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <rust/cxx.h>

#include <cstdint>

// macOS only. Safe wrappers over the C ABI of src/mac/bt_hfp_provider.m and
// src/mac/bt_hfp_support.m for src/bridge/native.rs. Everything except
// the JSON reads of prerequisites and endpoints must run on the main thread;
// the native side refuses (0, false or "") anywhere else.

// Starts one owned HF attempt for an exact binding. `session` is handed back
// to both callbacks unchanged, so a callback can only speak for the session
// that started it. Returns the native handle, or 0 when nothing was started.
uint64_t mac_hfp_start(rust::Str binding_json, uint64_t session,
                       rust::Fn<bool(uint64_t, rust::Slice<const uint8_t>)> lease,
                       rust::Fn<void(uint64_t, rust::Slice<const uint8_t>)> event);
// Asks that exact handle to return audio to the phone (irreversible).
bool mac_hfp_handback(uint64_t handle);
// Frees a retired handle; false while native closure is still unconfirmed.
bool mac_hfp_release(uint64_t handle);

rust::String mac_hfp_prerequisites();
// Already-paired hands-free gateways; ok is false when they can't be listed.
rust::String mac_hfp_paired_gateways(bool &ok);
rust::String mac_hfp_default_endpoints(bool &ok);
// Main thread, explicit user action only. Retains a plain Rust fn item for
// the process lifetime; changed runs on Main and must only refresh status.
bool mac_hfp_request_bluetooth(rust::Fn<void()> changed);
bool mac_hfp_request_microphone(rust::Fn<void()> changed);
void mac_hfp_pump_main_run_loop(double seconds);

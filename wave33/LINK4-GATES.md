# Wave 33 Android capability gates

Date: 2026-10-20

## Source checks

- `core/nectarlink-core/src/remote.rs`: Android's remote check accepts a peer advertising either `input.inject` or `deck.actions`. On the desktop, `check_allowed_with` emits the one-time `RemoteInputRequested` event when the per-device remote-input toggle is off. Windows can therefore support the first-use approval route before it advertises `input.inject`.
- `desktop/app/src/core_host.rs` (read-only): Windows always advertises `deck.actions` and advertises `input.inject` only when remote input is enabled for a paired device. Its macOS capability test verifies both `input.inject` and `deck.actions` are withheld.
- `core/nectarlink-core/src/features.rs`: `input.remote` requires desktop `input.inject`, while `input.deck` requires `deck.actions`. `camera.webcam` requires phone `camera.stream`, desktop `camera.virtual` and `addon.vcam`, Windows build 22000 or newer, and the webcam device toggle. The existing locked status exposes only the cheapest upgrade, so `grantPermission(camera)` alone did not prove that installing permission would make the backend usable.

## Changes

- The Control PC entry now accepts negotiated `input.remote` or `input.deck` while online. This preserves Windows' one-time approval route and keeps Home and Desk greyed out where both capabilities are withheld.
- The capability matrix now records a permission only when it is the sole unmet requirement in the current negotiated snapshot. Android exposes the webcam permission entry only when `camera` is that sole missing permission; missing desktop support, Windows build support, or the device toggle keeps it greyed out.
- No protocol opcode, capability advertisement, permission grant, or admission/authentication rule changed.

## Public API

- Rust core: `CapabilityMatrix::sole_missing_permission(&self, feature: &str) -> Option<Permission>`.
- UniFFI: `FeatureStatus::Locked` adds `sole_missing_permission: Option<String>`, populated with the permission ID only when it is the feature's sole unmet requirement. Generated Kotlin exposes this as `soleMissingPermission`.

## Checks

- Passed: `cargo test -p nectarlink-core --lib features::tests::webcam_permission_is_sole_missing_requirement_only_when_backend_is_ready` (1 test).
- Passed: `cargo check -p nectarlink-ffi --lib`.
- Passed: `rustfmt --edition 2024 --check core/nectarlink-core/src/features.rs core/nectarlink-ffi/src/lib.rs`.
- The Android unit test source was updated but not run because Gradle's unit-test task builds the Android app, which this dispatch explicitly prohibited. No app build or device validation was performed.

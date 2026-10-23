# Owned phone-hotspot preparation V3

Capability: `phone.hotspot.v3`. Both peers must allow the existing per-device `toggles` permission. Elevated Wi-Fi control or helper liveness alone never establishes hotspot authority. V3 explicitly guarantees that cancelling a join does not invoke an Android tether stop after submission; V2 providers are not admitted under that guarantee.

`phone.hotspot.prepare {request, network}` is a correlated RPC on the exact originating paired connection. `request` is a nonzero monotonic nonce; `network` is the UTF-8 SSID of the profile explicitly chosen and saved in the Mac's Wi-Fi settings. The SSID travels only over the authenticated encrypted peer channel and private authenticated helper socket; Debug redacts it. Passwords never cross this protocol.

The receiver records peer, its own session connection identity, request, cancellation state and a 40-second local preparation deadline before dispatching the platform callback. Its original-Node callback repeatedly checks `phone_hotspot_request_current`; it cannot bind an old event to a successor Node/session. `phone.hotspot.result {request,state}` returns `active`, `needs_settings`, `config_changed`, `failed` or `cancelled`. A successful join requires actual Wi-Fi tether interface readback and a stable configuration, never command receipt or shell exit.

`phone.hotspot.cancel {request}` retires only that logical request on the connection on which it arrives. Pre-cancel tombstones prevent delayed preparation after cancellation. Platform work runs outside the session receive loop so cancellation can be processed during an OS wait. Original-Node cancellation callbacks use `phone_hotspot_cancellation_current` to retire only the matching helper workflow. Successful retirement never sends cancellation.

## Existing authorized shell service route

The app first requires `HOTSPOT_PROTOCOL\t2` on the exact authenticated main H socket before opening any helper query. This is a syntax/version fence, not a capability promise. Old helpers receive no unknown first-line probes that could enter control-channel cleanup, and hotspot queries never replace or close the original input/clipboard/mirror connection.

The helper uses the existing ShellContext and actual UID-2000 permissions: TETHER_PRIVILEGED, ACCESS_NETWORK_STATE, and NETWORK_SETTINGS or OVERRIDE_WIFI_CONFIG. Root authorized exactly one read-only `WifiManager.getSoftApConfiguration` SystemApi getter adapter. No new bootstrap, permission grant, root route, configuration setter or shell hotspot command is added.

Before READY, the helper reads the current configuration, verifies its SSID against the expected saved network and derives a private HMAC with an ephemeral key. Only fingerprints and bounded logical owner/interface/request state are retained. Configuration objects, SSIDs, passphrases, exceptions and payloads are never stringified into diagnostics, placed in argv or written to files. Raw parcel byte arrays are cleared after hashing.

Before GO, the app rechecks the original Node, pair, session and request. Under actual TETHER_PRIVILEGED authority, typed `TetheringRequest.Builder(TETHERING_WIFI).build()` asks Android to use its current persistent configuration. No stale snapshot, replacement SSID/password or configuration setter is supplied.

A cold-start success requires the start callback, actual Wi-Fi tether readback, known callback-configuration consistency and a fresh matching configuration fingerprint. The normal app independently observes Wi-Fi tether state. Already-active reuse is checked without acquiring any shutdown authority. These observations establish join readiness; they are not an OS-owned handle against another actor.

## Cancellation leaves the phone hotspot on

The helper opcode is `CANCEL_JOIN`, deliberately distinct from V2's unsafe `CANCEL` opcode. V3 never sends a request that could invoke V2's tether-stop path; an older helper may reject the join-only opcode, with no fallback. V3 helpers reject the old opcode. The authenticated `HOTSPOT_PROTOCOL\t2` fence remains intact.

Cancellation records the exact logical owner and prevents a later pending start before submission. If Android activation has already been submitted, the helper returns `ACTIVATION_SUBMITTED`; it retires the workflow and performs no Android stop, rollback, configuration change or alternate off command. No lease is synthesized from configuration fingerprints, interface observations or a successful start callback, because those cannot establish exclusive OS ownership against concurrent users/privileged callers.

The UI states that cancel join leaves the phone hotspot on after activation has been submitted and instructs the user to turn it off manually in phone Settings. A Wi-Fi association already submitted to macOS may also finish. Session replacement, unpair, toggle revocation, expiry and Node/helper replacement still fail subsequent request/association checks. Cancelling Mac joining never closes the source protocol or input/clipboard/mirror connection.

## Native Mac joining and evidence boundary

The Mac stores the selected SSID only in a non-synchronizing Keychain item and obtains the saved protected-network password through the Wi-Fi Keychain API. The password remains local in memory, never argv or peer messages. Location access is requested only by the explicit setup action. Denied discovery, disabled Wi-Fi, unreadable Keychain credentials, unsupported/open security and association failures are precise errors. Native association rechecks ownership after scans/Keychain authorization and immediately before association.

The profile must be saved once through system Wi-Fi settings. Joining changes this computer's Wi-Fi only after the user's tap. Actual Android privilege/start, carrier Internet, native association and QUIC/path continuity remain separate Root runtime gates; source and synthetic tests do not establish them.

# Mac call audio adapter

This is an experimental native HFP setup, attempt and handback path. It is not
qualified full-duplex calling. `hfpAvailable` and `hfpConnected` remain false;
an observed Bluetooth service or SCO link does not establish that the Mac's
microphone and speaker carry a conversation or that handset audio resumed.

The Qt interface uses `PhoneCall.pcAudio*`. Bluetooth and microphone requests,
listing paired devices, selecting a phone, starting an attempt and returning
audio are explicit clicks. Opening the page must not prompt, scan, connect or
start audio. A Bluetooth name is display text, never identity.

An attempt binds the trusted Waypair device/key, explicitly selected paired
Bluetooth address, current microphone/speaker identities, sole answered call
and call generation. Changes invalidate its lease. Stop is latched before
handback; an unconfirmed handback keeps ownership and blocks a replacement.
Ending the Bluetooth attempt must not hang up or unpair the phone.

`macCallAudioBindings` is an internal desktop-settings field, omitted when
empty and preserved by other settings saves, including on Windows. It grants
no phone-side screen control and never rearms the mirroring Stop fence.

## Native boundary

- `src/mac/bt_hfp.rs`: application identity, lease, lifetime and status.
- `src/bridge/native.rs`: the existing bounded CXX FFI module. CXX-Qt's QML
  module requires its Rust bridge sources in one directory; do not add a
  second bridge directory or broaden the unsafe-code allowance.
- `cpp/mac_hfp.*`: native ABI and process-lifetime function-pointer callbacks.
- `src/mac/bt_hfp_provider.m`: HFP lifecycle and closure observations.
- `src/mac/bt_hfp_support.m`: explicit authorization requests and read-only
  prerequisites/device observations.

Microphone completion is dispatched to Main. The Bluetooth request independently
retains its weak delegate. Both notify a context-free Rust function that only
refreshes prerequisites and invalidates call UI; it does not start an attempt.
Shutdown suppresses late UI refresh. This does not establish broad OS permission
revocation enforcement.

Future normal bundles need the Bluetooth and microphone usage descriptions and
appropriate signing entitlements. Compilation and synthetic lease, settings,
closure and UI tests do not replace attended native checks: both audio
directions, handset continuation, pending/late callbacks, call replacement,
peer loss, permission responses and quit. Keep the feature unqualified until
those outcomes are observed on the intended hardware.

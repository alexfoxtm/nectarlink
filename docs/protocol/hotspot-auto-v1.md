# Automatic hotspot credential handoff

Automatic mode requires negotiated phone.hotspot.v3 and phone.hotspot.auto.v1. The existing prepare RPC adds auto_join=true and uses an empty network; legacy mode omits auto_join and keeps its saved-profile requirement. Result network is omitted unless automatic mode returns valid credentials with Active while its original owner remains current.

The requesting Mac advertises `phone.hotspot.auto.client.v1`; the phone checks
that client capability at admission and again before returning credentials.
The Mac does not advertise the phone-provider capability merely to receive it.

Credentials contain ssid, passphrase and security (wpa2 or wpa3). SSID is 1..32 UTF-8 bytes without NUL. Passphrase is 8..63 printable ASCII bytes. Invalid or missing credentials turn automatic Active into Failed. Credential Debug is redacted. UniFFI uses an opaque object so Kotlin outcome data-class toString delegates to an object identity instead of expanding the passphrase. Strings returned by explicit getters remain sensitive. No persistence or logging was added.

The existing one-pending-request reservation, live paired session, toggle, cancellation, monotonic request and 40/45-second deadlines are retained. Cancel does not stop tethering. The Android implementation must independently recheck the exact Node owner and same configuration fingerprint before returning verified active network credentials. The Mac implementation must hold credentials only for its current join and use native CoreWLAN association.

Paired-loopback tests cover automatic owned readback and cancellation after
submission with wrong-owner rejection. Privacy assertions also cover typed and
generic-envelope diagnostics. Software checks do not establish physical
tethering, Mac association, carrier connectivity, or device acceptance.

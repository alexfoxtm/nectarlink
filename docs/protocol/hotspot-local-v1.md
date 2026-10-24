# Local Android hotspot activation v1

One foreground Android Settings button activates or reuses the current protected hotspot. This is a local Activity operation, never a paired Mac request. Remote V3/automatic ownership, schema and credential admission stay unchanged.

The authenticated main H stream advertises HOTSPOT_LOCAL_PROTOCOL\t1. Elevated accepts it only on that exact current helper socket. LOCAL_PROBE returns capability only; LOCAL_PREPARE and LOCAL_CANCEL carry a random 256-bit Activity nonce, positive monotonic request and elapsedRealtime deadline no more than 40 seconds ahead. A typed LocalOwner cannot alias a remote Owner. Both domains share one pending helper reservation; local commands never ask for NETWORK credentials.

The app checks foreground lifetime, Activity generation, exact helper socket, deadline and banking pause before GO and before accepting results. ON_PAUSE/disposal invalidates local work and sends matching cancellation. Helpers recheck local deadline before submission and while waiting. Cancellation before submission prevents start; after submission it retires the workflow and leaves tethering on. No stop, rollback or shutdown lease exists.

Read-only protected-config capability checks require existing UID2000 permissions and current WPA2/WPA3 config. Activation retains current configuration and uses the existing typed tether request, callback and interface/config readback. UI additionally requires normal app-side Wi-Fi tether readback. Capability/launch/callback receipt alone is never success. No helper bootstrap, permission grant, configuration setter, credential display/log/storage, root or ADB route is added.

After activation iPhone/iPad uses native Wi-Fi network selection and the existing password visible in phone Settings. This does not implement Apple Instant Hotspot, remote iOS wake, guaranteed old-device/band support or verified carrier Internet. Physical Galaxy/iPhone/iPad association, real helper grants, provisioning, Internet routing and saved-network rejoin remain unperformed gates.

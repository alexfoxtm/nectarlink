// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

/**
 * Home peer status from real session metadata, mirroring the Mac badge rule.
 *
 * The triple comes from one `sessionDisplay` snapshot: the selected path
 * (`displayPath`: USB only when the live session's selected remote exactly
 * equals the CURRENT bridge address, relay when relay is selected), the ADB
 * tunnel origin (`usbMedium`: physical cable, network, or unknown), and the
 * proven route transport matched against this phone's own route table
 * (`routeTransport`: wifi, cellular, ethernet, usb, or unknown).
 *
 * Only `displayPath == "usb"` with a physical medium renders USB; the coarse
 * FFI `Link.usb` flag (true for any ADB tunnel, wired or wireless) is never
 * read here. Anything unproven renders Connected; relayed renders Away. The
 * phone's own bearer (wifiBars/cellBars) is never substituted for the peer's
 * path: only the session classification counts.
 */
internal enum class PeerOnlineLabel {
    AWAY,
    USB,
    WIFI,
    CELLULAR,
    ETHERNET,
    CONNECTED,
}

/** The closed proven-transport set the core reports. */
internal fun normalizePeerTransport(raw: String?): String? = when (raw) {
    "wifi", "cellular", "ethernet", "usb" -> raw
    else -> null
}

internal fun resolvePeerOnlineLabel(
    displayPath: String,
    usbMedium: String,
    routeTransport: String?,
): PeerOnlineLabel {
    if (displayPath == "relay") return PeerOnlineLabel.AWAY
    if (displayPath == "usb" && usbMedium == "physical") return PeerOnlineLabel.USB
    return when (normalizePeerTransport(routeTransport)) {
        "wifi" -> PeerOnlineLabel.WIFI
        "cellular" -> PeerOnlineLabel.CELLULAR
        "ethernet" -> PeerOnlineLabel.ETHERNET
        "usb" -> PeerOnlineLabel.USB
        else -> PeerOnlineLabel.CONNECTED
    }
}

/** Cellular display name: the proven network type when known, else the fallback label. */
internal fun cellularLabelName(cellType: String?, fallbackName: String): String {
    val type = cellType?.trim().orEmpty()
    return type.ifEmpty { fallbackName }
}

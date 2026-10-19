# The Waypair Link mark

This modified build ships as **Waypair Link** with Waypair's own mark, as
the upstream [trademark policy](../../../TRADEMARKS.md) requires of forks. It
is based on NectarLink; the NectarLink files one level up stay as upstream's
and are not used by the Waypair Link apps.

The mark: one pearl-white path, two rounded U shapes that cross once below a
loop, forming a W for two paired devices, on royal blue (`#164FA8` at the top
to `#063B86` at the bottom).

| File | What it is |
|---|---|
| `waypair-icon-v2.png` | The full-bleed 1254 × 1254 source artwork (no rounded corners baked in) |
| `waypair-icon-v2.provenance.json` | How the artwork was made and its original exports |

Taken unchanged from the legacy Waypair app (`assets/brand/` there, SHA-256
`6768e4b7…88e2`). The apps use it like this:

- Android: `res/drawable-nodpi/ic_launcher_waypair_v2.png` (the 1024 px
  export, SHA-256 `2dc2e205…1420`) in the adaptive launcher foreground on a
  `#063B86` background, plus the traced `ic_launcher_monochrome.xml` for
  themed icons and `ic_notification.xml`.
- Desktop: `desktop/app/src/mark.rs` draws the same path in code for the
  window, tray, toast and executable icons, and `Icons.mark` in
  `desktop/app/qml/Icons.qml` is its one-line version for the UI.

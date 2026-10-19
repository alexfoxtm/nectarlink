# The Waypair Link mark

This modified build ships as **Waypair Link** with Waypair's own mark, as
the upstream [trademark policy](../../../TRADEMARKS.md) requires of forks. It
is based on NectarLink; the NectarLink files one level up stay as upstream's
and are not used by the Waypair Link apps.

## Current mark: black and white (2026-10-19)

The mark: one white path, two rounded U shapes that cross once below a loop,
forming a W for two paired devices, on a black tile. Only neutral grayscale
is used — `#FFFFFF` glyph on `#000000`, with a neutral-gray rim and
antialiasing. No blue, honey, or tinted pixels; the apps' theme colors are
unchanged, only the logo changed.

The glyph geometry is the tested Waypair W from `desktop/app/src/mark.rs`
at commit `8c68bdcc`, restored unchanged; only its color constants changed.
It is drawn in code, so the canonical assets below and the runtime icons can
never drift apart.

| File | What it is |
|---|---|
| `waypair-icon-monochrome.svg` | The canonical monochrome app icon: white W on the black rounded tile |
| `waypair-glyph-monochrome.svg` | The white W alone on transparency, for single-color uses |
| `waypair-icon-monochrome-512.png` | `waypair-icon-monochrome.svg` geometry at 512 × 512, rendered from `mark.rs` |
| `waypair-icon-monochrome-1024.png` | The same at 1024 × 1024 |
| `AppIcon-monochrome.icns` | macOS icon (16–1024) built from the same renders; the default icon in `scripts/macos/package-app.sh` |
| `waypair-icon-monochrome.provenance.json` | How the monochrome assets were made, with source-path and neutral-color proof |

The apps use it like this:

- Desktop: `desktop/app/src/mark.rs` draws the path in code for the window,
  tray, toast and executable icons, and `Icons.mark` in
  `desktop/app/qml/Icons.qml` is the same centerline scaled onto the 24×24
  stroke-icon grid for the UI.
- Android: `res/drawable/ic_launcher_foreground.xml` (white W; the stroked
  glyph spans x 23.98–76.02, y 28.73–71.27 of the 100-unit canvas, inside
  the 66dp adaptive safe zone) on `ic_launcher_background.xml` (solid
  `#000000`), plus `ic_launcher_monochrome.xml` for themed icons and
  `ic_notification.xml`, both single-color silhouettes of the same W.

## Previous mark: blue v2 (retained, not used)

The earlier mark was the same shape in pearl white on royal blue (`#164FA8`
at the top to `#063B86` at the bottom). Its files stay unchanged, with their
original provenance; they are not the current logo and were not overwritten
as if they were the monochrome source.

| File | What it is |
|---|---|
| `waypair-icon-v2.png` | The full-bleed 1254 × 1254 blue source artwork (no rounded corners baked in) |
| `waypair-icon-v2.provenance.json` | How the blue artwork was made and its original exports |
| `AppIcon.icns` | The blue v2 macOS icon |

Taken unchanged from the legacy Waypair app (`assets/brand/` there, SHA-256
`6768e4b7…88e2`).

OneUIProject oneui-icons
======================

Copyright (c) 2022 Yanndroid & BlackMesa123
MIT license, retained in LICENSE.
Source: https://github.com/OneUIProject/oneui-icons
Pinned commit: 3808bd6965251e7616b1a0a286c75f5d533715e7

The original/ directory contains unchanged upstream Android VectorDrawables.
The svg/ directory contains faithful SVG conversions with primary icon ink rendered black.
Icons.qml embeds these documents with a primary-ink placeholder so Image can render caller colours.
No XML/SVG geometry is flattened or replaced by another icon family.

Waypair derivatives: micOff adds only the last top-level path (the exact mute slash)
from ic_oui_audio_system_mute.xml, scaled from its 24 viewport to voice's 72 viewport.
rewind10 and forward10 retain the undo/redo vectors and add visible sans-serif 10 text.
callEnd rotates the filled phone vector 135 degrees about its authored viewport centre.
These are OneUI-derived app assets, not upstream assets. Other semantic equivalents
and selected counterparts follow manifest.json's mapping notes.

The OneUIProject artwork is used under its MIT license. This does not grant rights
to Samsung trademarks or imply Samsung endorsement. The Waypair brand mark is unchanged.

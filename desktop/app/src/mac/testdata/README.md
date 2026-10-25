# Synthetic H.264 fixture

`red-32x32.annexb` is an original, one-frame solid-red 32x32 test pattern,
created locally without camera, microphone, screen capture or personal data.
SHA-256: 8013c4ec0b809193701da4f9fca1d701601508f06776adf8f70bbe35cc823b45.

Generated with FFmpeg/libx264 baseline, yuv420p, no B frames, repeated headers,
AUD, BT.709 limited-range metadata; filter_units removes the SEI NAL. The encoder
is a development tool and is not bundled with Waypair. Offline reference decode
produced one 4096-byte BGRA frame with first pixel [0, 0, 252, 255].

Generation command:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i color=c=red:size=32x32:rate=1 \
  -frames:v 1 -pix_fmt yuv420p -c:v libx264 -profile:v baseline \
  -preset ultrafast -tune zerolatency \
  -x264-params keyint=1:bframes=0:repeat-headers=1:aud=1 \
  -color_range tv -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
  -bsf:v filter_units=remove_types=6 -f h264 red-32x32.annexb
```

This test data is dedicated to the public domain under CC0-1.0.

`red-128x128.annexb` uses the same command with `size=128x128` and
`-level:v 3.1`. It exercises the production hardware-preferred sink; this Mac
refuses the 32x32 format when hardware is required with OSStatus -12911,
while the existing software-selected 32x32 decoder fixture remains separate.
Original synthetic data, CC0-1.0, SHA-256: 10d5435dda2f47dfd5de159484ce34c1f464bb00361515d01aabb8e62ea83b43.

## Translation resource fixture

`i18n-fixture-de.ts` contains one synthetic German context, isolated from
application text. `i18n-fixture-de.qm` is generated from that reviewed source
with `lrelease`; it contains the `de_DE` translation `Pfad`.
Regenerate it with:

```sh
lrelease i18n-fixture-de.ts -qm i18n-fixture-de.qm
```

The production-loader test creates only temporary `Contents/MacOS` and
`Contents/Resources` data directories. It uses Qt Core without a GUI, bundle
identity, registration, or device, and checks both loading and English fallback. The fixture locale is not offered
in Settings or shipped as an application catalog.

# Upstream and fork policy

Alex's decision, 2026-10-19:

- If NectarLink welcomes contributions, official NectarLink is the intended primary product. Keep this synchronized fork as fallback for rejected or not-yet-accepted features.
- Verify the local macOS port and useful Android improvements first, then propose focused reusable fixes upstream with original attribution and DCO.
- **R2 updater code stays in this fork only.** Exclude the Android R2 client, Mac R2/Sparkle helper integration, feeds, publisher, owner signing configuration and their tests from upstream contributions.
- Keep branding and owner distribution changes separate from generic clipboard, Keychain, lifecycle, platform and Android fixes. Rejected features remain isolated fork changes while the base follows upstream.
- Cloud stays independent. Preserve release keys, data and immutable artifacts. A source merge is not installation or OTA acceptance.

The working branch is `waypair/macos-port`; `origin` is `alexfoxtm/nectarlink`, and `upstream` is `thtbee/nectarlink`.

# Upstream and fork policy

Alex's decision, 2026-10-19:

- If NectarLink welcomes contributions, official NectarLink is the intended primary product. Keep this synchronized fork as fallback for rejected or not-yet-accepted features.
- Verify the local macOS port and useful Android improvements first, then propose focused reusable fixes upstream with original attribution and DCO.
- **R2 updater code stays in this fork only.** Exclude the Android R2 client, Mac R2/Sparkle helper integration, feeds, publisher, owner signing configuration and their tests from upstream contributions.
- Keep branding and owner distribution changes separate from generic clipboard, Keychain, lifecycle, platform and Android fixes. Rejected features remain isolated fork changes while the base follows upstream.
- Cloud stays independent. Preserve release keys, data and immutable artifacts. A source merge is not installation or OTA acceptance.

## Name, logo and version (personal use only)

Alex's later decision, 2026-10-19, replaces the Waypair Link rebrand:

- These builds are for Alex's personal use only. They keep the NectarLink name and the upstream Waggle Dance mark (`assets/brand/`), as in upstream `e71dee41`; they are not distributed as a product of their own.
- Every build says it is a local build: Android About reads "NectarLink <version> · personal local build, not an official release", and desktop About has a "Local build for personal use" row stating it is not an official NectarLink release. Nothing implies an official upstream release.
- The version follows upstream (`0.1.4`) with a local build suffix; Root sets it in the build, not in branding files.
- The macOS bundle defaults to the name `NectarLink` with `assets/brand/AppIcon.icns`, made with `sips` and `iconutil` from upstream's `assets/brand/icon-512.png` (16 to 512 px; no 1024 px slot rather than upscale).
- Identity stays as installed: Android `com.waypair` package overrides and signers, the Mac bundle id, data folders, keys and R2 feeds are unchanged; no package, data or key migration. The native application and shelf titles are NectarLink; shelf lookups were updated together.
- R2 stays fork-only and Cloud stays separate, as above.

The working branch is `waypair/macos-port`; `origin` is `alexfoxtm/nectarlink`, and `upstream` is `thtbee/nectarlink`.


## Latest product decision — Waypair Link, monochrome mark

Alex's later instruction on 2026-10-19 supersedes the personal NectarLink branding and official-upstream-primary preference above. The product is our independently maintained **Waypair Link** Android and macOS fork, built from NectarLink and regularly incorporating reviewed macOS updates from saivinay1805 and useful canonical upstream changes. Keep our feature delta and private R2 delivery under our control; contributions upstream remain separately prepared generic patches.

Use the original Waypair W mark in black and white on both platforms. Only the logo changes color; existing app themes remain. About says “Independently maintained fork” without prominent NectarLink branding or a promotional attribution paragraph. Original author copyrights, attribution, license files and source notices remain available in Open-source Licenses and the source. This is a UI branding change, not a claim to have authored upstream code. Preserve package/bundle IDs, cryptographic identities, pairing, data, permissions, version monotonicity and the existing dedicated update channel paths/keys. Cloud stays separate. Alex will install the Galaxy APK; Root publishes both qualified builds to R2, installs the Mac build and removes only the confirmed obsolete NectarLink app bundle, never its data.

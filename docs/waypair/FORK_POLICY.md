# Waypair Link fork policy

Alex's latest decision, 2026-10-19, supersedes the earlier personal NectarLink branding and official-upstream-primary preference.

## Product and branding

- Develop **Waypair Link** as our independently maintained Android and macOS product, using the actual NectarLink Qt Quick/Rust desktop and matching Android codebase. No legacy Swift expansion or compatibility bridge. Waypair Cloud stays independent.
- Use the original Waypair W mark in **black and white** on both platforms. Existing application theme colors remain; only the logo becomes monochrome. Keep the upstream brand assets unchanged for source history, not as our app icon.
- About says **Independently maintained fork**, without prominent NectarLink branding or a promotional attribution paragraph. Original copyrights, authorship, attribution, source notices and full license files remain available in Open-source Licenses and the source. Do not imply that we authored upstream code or that this is an official upstream release.
- These are Alex's personal builds. Version numbers remain monotonic; the current workspace follows 0.1.4 with local build markers. Branding does not change package IDs, data locations or cryptographic identity.

## Upstream synchronization and contributions

- Upstream welcomed contributions in [thtbee's 2026-10-19 reply](https://github.com/thtbee/nectarlink/pull/24#issuecomment-6278184327). Follow that coordination: prepare small, focused PRs; wait for the macOS port in #24 to merge before submitting macOS follow-ups; propose Android changes, including Fold layouts, in an issue and agree on the approach before upstream implementation. Existing local experiments are not upstream-approved work.
- Keep the upstream platform layout and capability negotiation intact: macOS code lives in `desktop/app/src/mac/`, shared modules remain single files with platform `cfg` blocks, and unsupported features are not advertised. Generic fixes must preserve Windows and Linux behavior. A passing local Mac build does not prove Windows/Linux checks.
- Prepare contribution commits on a clean upstream-based branch, with original authorship, applicable licenses, legitimate DCO sign-offs and the checks required by `CONTRIBUTING.md`. Do not rewrite published fork history or submit the whole product branch. Split mixed branding/delivery and generic hunks before review.
- Before each development round, check canonical main/releases and saivinay1805's macOS PR24/feature branch using the provenance tooling. Review exact SHAs, preserve original author/history and notices, resolve overlaps, and run relevant checks before adopting changes through an integration checkout.
- Keep our feature changes in focused commits so useful Android, desktop and macOS updates remain easy to incorporate. Waypair Link remains our product; prepare reusable generic contributions upstream separately.
- **All R2 updater code stays exclusively in our fork.** Never include Android R2 clients, Mac Sparkle helper/integration, feeds, owner signing configuration, publisher or their tests in upstream contributions. Split mixed files/hunks before preparing a contribution.
- The working branch is `waypair/macos-port`; `origin` is `alexfoxtm/nectarlink`, `upstream` is `thtbee/nectarlink`. No maintainer message or upstream PR submission is implied by routine synchronization.

## Identity, delivery and acceptance

- Preserve Android `com.waypair`, Mac `com.waypair.mac`, release signers, Keychain/Keystore aliases, encrypted identity, saved pairing, app data, permissions, update keys and immutable release objects. Keep existing dedicated channel paths even when their internal namespace says `nectar`.
- A renamed app is an in-place product update, not a key/data migration. Do not import or clear old trust records. Cloud stays in its own sandbox and release channel.
- Build/test/sign/publish qualified personal candidates locally to R2. Keep source, checks, artifact qualification, installed version, public readback, OTA and paired-device acceptance distinct.
- Alex installs the Galaxy APK himself. Root installs the branded Mac build and removes only the confirmed obsolete NectarLink app bundle after replacement qualification; preserve all saved app data and keys.

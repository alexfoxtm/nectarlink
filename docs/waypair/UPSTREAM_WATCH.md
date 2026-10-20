# Ongoing macOS upstream

PR [thtbee/nectarlink#24](https://github.com/thtbee/nectarlink/pull/24), from `saivinay1805/nectarlink:feature/macos-port`, is the ongoing macOS source of changes and maintainer guidance. Canonical `thtbee/nectarlink:main` remains the Android/shared upstream.

Before each development batch, inspect new PR commits and discussion and compare exact changed files before implementing overlapping work. Use the existing provenance tooling and check at most hourly unless Alex explicitly requests a fresh check. Record the observation time, upstream SHA, adopted SHA and local delta separately. A live PR head is not automatically adopted or runtime-qualified.

Latest live API check on 2026-10-20: PR24 open, head `749b7fd64a2dac1e8bac1b8fd6bfbb997ff9ee91`, base/main `5bdcadbf9e2c4d3ca55111829d840574028afbcb`. Its nine commits include a rebase; the older adopted full port pin `9c0f13d7` is not its ancestor. The fork adopted selected time/free-space helpers from the later head, not the entire rebased PR. Re-review the complete delta before advancing provenance.

Keep generic fixes and tests focused, DCO-signed and attributable. Follow the maintainer's request to wait for PR24 before proposing overlapping macOS contributions, and agree Android feature approaches through an issue first. No upstream message, issue or PR is authorized by this document.

R2 updater clients/helpers, feeds, owner configuration, publisher and their tests stay fork-only. Preserve upstream copyright and full license/source notices. Keep Cloud independent.

## Current transport requirement

Alex rejected the neutral ADB badge. The Mac must distinguish the actual Wi-Fi, cellular/LTE and physical USB connection. The existing upstream code classifies all ADB tunnels as USB, including wireless ADB. Reuse upstream UI behavior where valid, but do not use signal bars or a serial-name guess as proof of the actual session route. Preserve routing, authentication, pairing and admission while repairing display metadata.

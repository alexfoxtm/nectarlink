# Waypair 0.2.0 (119)

The current Link Mac, Link Android and separate Cloud release share version 0.2.0 and build 119. Their update feeds and application IDs remain separate.

## Client capabilities

- Camera preview without our native extension, clean external-capture window (Mac).
- Native network-change detection for reconnect (Mac).
- Configured command palette keyboard shortcut (Mac).
- Open selected local sync folder in Finder (Mac).
- Progress snapshots in native notification posts (Mac).
- Pinned and text/image clipboard-history filters (Mac).
- Retry failed/expired items for selected phone (Mac).
- Search and filter queued transfers (Mac).
- Search loaded conversation with match navigation (Mac).
- Copy selected message text (Mac).
- Selected-phone notification history and search feedback (Mac).
- Loaded photo filename search and media filter (Mac).
- Invert displayed selection with truthful selection summary (Cloud).
- Sort reminders by due date or priority (Cloud).
- Chronological ordering of loaded photo search matches (Cloud).
- Search and status-filter offline folders (Cloud).

Camera preview is separate from a system camera device. External OBS virtual camera setup and application compatibility require their own qualification. The Waypair camera extension remains unavailable without matching authorized provisioning.

General arbitrary-app control flags remain disabled. Phone grants, native notification actions, physical folding and reconnect checks, and audible call duplex/handback require their existing device acceptance. No new account action or OS permission is implied by this release.

The release also updates ctutils to 0.4.5, uses bounded HTTPS-only atomic downloads on Mac, and scopes Cloud upload-failure notices to the account and document.

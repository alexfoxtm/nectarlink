// SPDX-License-Identifier: GPL-3.0-or-later
//
// WaypairUpdater — tiny native Sparkle 2.10 host for the Nectar-based
// Waypair Link Mac app.
//
// Why a helper: the desktop core is Rust/Qt. Sparkle is an AppKit
// framework, so a small native process owns `SPUUpdater` and its standard
// user driver; Rust merely spawns this helper with `--check` when the user
// asks for an update check (`desktop/app/src/updater.rs`,
// `macos_user_check`). Sparkle does all feed fetching, Ed25519 appcast
// verification (against `SUPublicEDKey` in the host bundle), version
// comparison, download, and installation — this file adds no download or
// trust logic of its own, and never trusts a bare checksum.
//
// Host bundle: the `.app` that contains this helper (found by walking up
// from the executable path), used as both host and application bundle.
// `SUFeedURL` and `SUPublicEDKey` are read by Sparkle from that bundle's
// Info.plist; if either is missing the helper reports the updater as
// unavailable and exits non-zero instead of guessing a feed or a key.
//
// Root builds and signs this file (sole compiler), e.g.:
//   swiftc -O -framework Sparkle \
//     -F <dir-containing-Sparkle.framework> \
//     -Xlinker -rpath -Xlinker @executable_path/../Frameworks \
//     scripts/macos/WaypairUpdater.swift -o WaypairUpdater
// The framework itself is bundled by scripts/macos/package-app.sh
// (--sparkle-framework; sourced from the existing verified Sparkle 2.10.0
// install, never downloaded by the script).

import AppKit
import Foundation
import Sparkle

private func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("WaypairUpdater: \(message)\n".utf8))
    exit(2)
}

/// The `.app` bundle containing this executable
/// (`…/<App>.app/Contents/MacOS/WaypairUpdater`).
private func hostBundle() -> Bundle {
    var url = Bundle.main.executableURL ?? URL(fileURLWithPath: CommandLine.arguments[0])
    while url.pathExtension != "app" {
        let parent = url.deletingLastPathComponent()
        guard parent.path != url.path else {
            fail("could not locate the host .app bundle from \(url.path)")
        }
        url = parent
    }
    guard let bundle = Bundle(url: url) else {
        fail("could not open host bundle at \(url.path)")
    }
    return bundle
}

private final class Delegate: NSObject, SPUUpdaterDelegate {
    /// Set when Sparkle found something installable: the standard user
    /// driver is presenting it, so the helper must stay alive until the
    /// update cycle finishes instead of exiting at "not found" handlers.
    var foundUpdate = false

    func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
        foundUpdate = true
        print("WaypairUpdater: update \(item.displayVersionString) is available")
    }

    func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: Error) {
        print("WaypairUpdater: no update available (\(error.localizedDescription))")
        NSApp.terminate(nil)
    }

    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
        if let error {
            FileHandle.standardError.write(
                Data("WaypairUpdater: update check failed: \(error.localizedDescription)\n".utf8))
            NSApp.terminate(nil)
        } else if !foundUpdate {
            NSApp.terminate(nil)
        }
        // With an update found, the standard user driver owns the flow
        // (download/install/relaunch); exiting here would kill its UI.
    }
}

@main
private enum Main {
    static func main() {
        guard CommandLine.arguments.contains("--check") else {
            fail("usage: WaypairUpdater --check")
        }
        let bundle = hostBundle()
        // Truthful availability: Sparkle needs both a feed and the pinned
        // Ed25519 public key in the host bundle. Missing means this build
        // simply cannot check — say so, never fall back to another feed.
        guard let feed = bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String, !feed.isEmpty else {
            fail("SUFeedURL is missing from the host bundle; updates are unavailable in this build")
        }
        guard let key = bundle.object(forInfoDictionaryKey: "SUPublicEDKey") as? String, !key.isEmpty else {
            fail("SUPublicEDKey is missing from the host bundle; updates are unavailable in this build")
        }
        _ = feed
        _ = key

        let app = NSApplication.shared
        let delegate = Delegate()
        let updater = SPUUpdater(
            hostBundle: bundle,
            applicationBundle: bundle,
            userDriver: SPUStandardUserDriver(hostBundle: bundle, delegate: nil),
            delegate: delegate)
        do {
            try updater.start()
        } catch {
            fail("could not start the updater: \(error.localizedDescription)")
        }
        // User-initiated check: Sparkle shows progress/UI itself and only
        // ever installs a version newer than the host bundle's, from the
        // bundle's configured (dedicated Nectar)
        // feed — never the legacy Swift app's appcast items.
        updater.checkForUpdates()
        app.run()
    }
}

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
// comparison (against the host's `CFBundleVersion`), download, and
// installation — this file adds no download or trust logic of its own, and
// never trusts a bare checksum.
//
// Layout: its own LSUIElement app, `<App>.app/Contents/Helpers/
// WaypairUpdater.app`, with the bundle identifier `<host id>.updater`.
// Sparkle's installer picks the process to quit and relaunch by the host's
// bundle identifier and path, so the helper must not share either; this is
// the arrangement of Sparkle's own sparkle-cli (host bundle = the app to
// update, not the helper's main bundle).
//
// Root builds and signs this file (sole compiler), e.g.:
//   swiftc -parse-as-library -O -framework Sparkle \
//     -F <dir-containing-Sparkle.framework> \
//     -Xlinker -rpath -Xlinker @executable_path/../../../../Frameworks \
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

/// The app this helper updates: the `.app` enclosing
/// `<App>.app/Contents/Helpers/WaypairUpdater.app`, and only when this
/// helper really is that nested bundle with the identifier `<host id>.updater`.
private func hostBundle() -> Bundle {
    let own = Bundle.main.bundleURL.resolvingSymlinksInPath()
    var url = own.deletingLastPathComponent()
    while url.pathExtension != "app" {
        let parent = url.deletingLastPathComponent()
        guard parent.path != url.path else {
            fail("no host .app encloses \(own.path)")
        }
        url = parent
    }
    let expected = url.appendingPathComponent("Contents/Helpers/WaypairUpdater.app", isDirectory: true)
    guard expected.standardizedFileURL.path == own.standardizedFileURL.path else {
        fail("must run as <App>.app/Contents/Helpers/WaypairUpdater.app, not \(own.path)")
    }
    guard let host = Bundle(url: url), let hostID = host.bundleIdentifier, !hostID.isEmpty else {
        fail("the host bundle at \(url.path) has no bundle identifier")
    }
    guard Bundle.main.bundleIdentifier == "\(hostID).updater" else {
        fail("the helper's bundle identifier must be \(hostID).updater")
    }
    guard let executable = host.executableURL, FileManager.default.isExecutableFile(atPath: executable.path) else {
        fail("the host bundle at \(url.path) has no executable")
    }
    return host
}

@MainActor
private final class Delegate: NSObject, SPUUpdaterDelegate {
    /// A short-lived, user-started check: Rust schedules nothing here, so
    /// Sparkle must not ask about automatic checks.
    @objc(updaterShouldPromptForPermissionToCheckForUpdates:)
    func updaterShouldPromptForPermissionToCheck(forUpdates updater: SPUUpdater) -> Bool {
        false
    }

    /// Sparkle calls this only once its user driver is done: after the
    /// up-to-date or error alert is acknowledged, after Dismiss, Skip, or
    /// Later, and after an install. Every path ends the helper here.
    @objc(updater:didFinishUpdateCycleForUpdateCheck:error:)
    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
        guard let error = error as NSError? else {
            exit(0)
        }
        // SUNoUpdateError, SUInstallationCanceledError,
        // SUInstallationAuthorizeLaterError (SUErrors.h): outcomes, not failures.
        if error.domain == SUSparkleErrorDomain, [1001, 4007, 4008].contains(error.code) {
            exit(0)
        }
        FileHandle.standardError.write(
            Data("WaypairUpdater: update check failed: \(error.localizedDescription)\n".utf8))
        exit(1)
    }
}

@main
@MainActor
enum Main {
    static func main() {
        guard CommandLine.arguments.contains("--check") else {
            fail("usage: WaypairUpdater --check")
        }
        let bundle = hostBundle()
        // Sparkle needs both a feed and the pinned Ed25519 public key in the
        // host bundle. Missing means this build simply cannot check — say
        // so, never fall back to another feed.
        guard let feed = bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String,
              let feedURL = URL(string: feed), feedURL.scheme == "https", feedURL.host != nil else {
            fail("the host bundle has no https SUFeedURL; updates are unavailable in this build")
        }
        guard let key = bundle.object(forInfoDictionaryKey: "SUPublicEDKey") as? String, !key.isEmpty else {
            fail("SUPublicEDKey is missing from the host bundle; updates are unavailable in this build")
        }

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
        updater.checkForUpdates()
        // SPUUpdater holds its delegate weakly; both must outlive the run loop.
        withExtendedLifetime((delegate, updater)) {
            app.run()
        }
    }
}

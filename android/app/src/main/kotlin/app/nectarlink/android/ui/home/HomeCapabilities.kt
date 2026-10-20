// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import app.nectarlink.android.core.Device
import app.nectarlink.core.FeatureStatus

/**
 * Negotiated matrix IDs behind the Home PC-card actions (`core/features.rs`
 * `FEATURES`). Gates use [Device.has] (Available only), with the Control PC
 * approval-flow and Webcam permission-path exceptions below.
 */
internal const val HOME_FEATURE_INPUT_REMOTE = "input.remote"
internal const val HOME_FEATURE_INPUT_DECK = "input.deck"
internal const val HOME_FEATURE_CAMERA_WEBCAM = "camera.webcam"
internal const val HOME_FEATURE_PC_BROWSE = "files.pc_browse"

/**
 * Control PC runs where the desktop negotiates `input.inject`. While
 * Windows waits for the user's first remote-input request before announcing
 * that capability, `remote::check` accepts `deck.actions` to trigger the
 * one-time approval flow. Mac withholds both capabilities.
 */
internal fun Device.canControlPc(): Boolean =
    online && (has(HOME_FEATURE_INPUT_REMOTE) || has(HOME_FEATURE_INPUT_DECK))

/**
 * Deck runs only where the desktop negotiates `deck.actions` (`deck::press`
 * refuses anything else; Windows always announces it, Mac withholds it).
 * The Deck page's local fallback layout and audio section are untouched;
 * this gates only the Home entry.
 */
internal fun Device.canOpenDeck(): Boolean = online && has(HOME_FEATURE_INPUT_DECK)

/** Browse PC files only where the desktop advertises `storage.read`. */
internal fun Device.canBrowsePcFiles(): Boolean = online && has(HOME_FEATURE_PC_BROWSE)

/**
 * Webcam runs only where the desktop negotiates `camera.virtual` plus the
 * virtual-camera add-on on a new enough build. While the feature is Locked
 * only on the phone-side camera permission, the entry stays reachable: the
 * Webcam screen itself asks for that permission and the matrix flips to
 * Available once granted. Matrix metadata proves the phone permission is the
 * sole unmet requirement, so a missing desktop add-on keeps the button greyed
 * out too.
 */
internal fun Device.canOpenWebcam(): Boolean =
    online && (has(HOME_FEATURE_CAMERA_WEBCAM) || webcamLockedOnPhonePermission())

/**
 * Whether `camera.webcam` is Locked solely on the phone camera permission
 * (`UpgradeAction::describe` gives `("grantPermission", "camera")`; see
 * `Permission::Camera::as_str` in `core/features.rs`). No desktop
 * requirement may be the blocker: those never clear from the phone.
 */
internal fun Device.webcamLockedOnPhonePermission(): Boolean =
    features.any { feature ->
        feature.id == HOME_FEATURE_CAMERA_WEBCAM &&
            (feature.status as? FeatureStatus.Locked)?.let { locked ->
                locked.soleMissingPermission == "camera" &&
                    locked.upgrade.action == "grantPermission" && locked.upgrade.target == "camera"
            } == true
    }

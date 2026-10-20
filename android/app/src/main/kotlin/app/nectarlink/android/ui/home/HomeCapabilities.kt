// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import app.nectarlink.android.core.Device
import app.nectarlink.core.FeatureStatus

/**
 * Negotiated matrix IDs behind the Home PC-card actions (`core/features.rs`
 * `FEATURES`). A gate uses [Device.has] (Available only): these features
 * declare no weaker `partial` set, so `usable` would add nothing.
 */
internal const val HOME_FEATURE_INPUT_REMOTE = "input.remote"
internal const val HOME_FEATURE_INPUT_DECK = "input.deck"
internal const val HOME_FEATURE_CAMERA_WEBCAM = "camera.webcam"

/**
 * Control PC runs only where the desktop negotiates `input.inject`
 * (`remote::check` refuses anything else as Unsupported; `core_host.rs`
 * withholds the cap on Mac, where it stays Locked, never Available).
 */
internal fun Device.canControlPc(): Boolean = online && has(HOME_FEATURE_INPUT_REMOTE)

/**
 * Deck runs only where the desktop negotiates `deck.actions` (`deck::press`
 * refuses anything else; Windows always announces it, Mac withholds it).
 * The Deck page's local fallback layout and audio section are untouched;
 * this gates only the Home entry.
 */
internal fun Device.canOpenDeck(): Boolean = online && has(HOME_FEATURE_INPUT_DECK)

/**
 * Webcam runs only where the desktop negotiates `camera.virtual` plus the
 * virtual-camera add-on on a new enough build. While the feature is Locked
 * only on the phone-side camera permission, the entry stays reachable: the
 * Webcam screen itself asks for that permission and the matrix flips to
 * Available once granted. Any other missing requirement (Mac withholds every
 * desktop side) keeps the button greyed out.
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
            (feature.status as? FeatureStatus.Locked)?.upgrade?.let {
                it.action == "grantPermission" && it.target == "camera"
            } == true
    }

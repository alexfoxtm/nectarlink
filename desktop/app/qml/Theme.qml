// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
import app.nectarlink

// Mac uses neutral surfaces and monochrome actions. Legacy style preferences
// remain stored for compatibility; light/dark follows colorMode.
QtObject {
    id: theme

    readonly property bool onMac: Qt.platform.os === "osx" || Qt.platform.os === "macos"
    readonly property bool hive: false
    readonly property bool graphite: false
    readonly property bool glass: onMac
    readonly property bool dark: Preferences.colorMode === "dark"
                                 || (Preferences.colorMode === "system" && AppController.systemDark)
    readonly property bool amoled: false
    readonly property bool reduceMotion: AppController.reduceMotion

    // ---- Hive's comb ----
    readonly property var comb: Tokens.data.themes.hive.comb
    // Breathes and pulses: the user's setting, never with reduced motion or
    // battery saver (it holds still then).
    readonly property bool combMotion: hive && Preferences.combMotion && !reduceMotion && !AppController.powerSaver
    readonly property color combLine: Qt.rgba(primary.r, primary.g, primary.b,
                                              dark ? comb.lineAlphaDark : comb.lineAlphaLight)
    readonly property color combGlow: onMac ? Qt.rgba(primary.r, primary.g, primary.b, dark ? comb.glowAlphaDark : comb.glowAlphaLight)
        : dark ? Qt.rgba(1.0, 0.62, 0.18, comb.glowAlphaDark)
                                           : Qt.rgba(1.0, 0.72, 0.37, comb.glowAlphaLight)
    // One clock for every comb in the app (seconds), so two combs never draw
    // twice as often. The main window runs it while it's in front.
    property real combTime: 0
    property bool combClockRunning: false
    // A comb is on screen outside Hive (the invitation to try it).
    property bool combPreviewShown: false
    property double _combSince: 0
    property real _combBase: 0
    readonly property Timer _combClock: Timer {
        interval: theme.comb.frameMs
        repeat: true
        running: theme.combClockRunning
                 && (theme.combMotion || (theme.combPreviewShown && !theme.reduceMotion && !AppController.powerSaver))
        onRunningChanged: {
            if (running)
                theme._combSince = Date.now()
            else
                theme._combBase = theme.combTime
        }
        onTriggered: theme.combTime = theme._combBase + (Date.now() - theme._combSince) / 1000
    }

    readonly property var bloomSeeds: Tokens.data.themes.bloom.seeds
    // Retained seed fallback for stored preference compatibility.
    readonly property string fallbackSeed: "ocean"
    // Colors made from the desktop wallpaper (or null until they're known),
    // in the shape of a preset seed.
    readonly property var wallpaperSeed: AppController.wallpaperColors !== ""
        ? JSON.parse(AppController.wallpaperColors) : null
    // Material You color scheme extracted from the currently playing media artwork.
    readonly property var artworkSeed: {
        if (MediaList.activeColors === "")
            return null
        try {
            return JSON.parse(MediaList.activeColors)
        } catch (e) {
            return null
        }
    }
    readonly property string artworkUrl: MediaList.activeArt
    readonly property bool isArtworkSeed: Preferences.seed === "artwork"
    readonly property bool artworkAmbientActive: false
    readonly property bool ambientBlurActive: glass
    // Material You color schemes from paired phones (`[{ id, key, name, label, hasAccent, scheme }]`).
    readonly property var phoneSeeds: AppController.phoneColors !== ""
        ? JSON.parse(AppController.phoneColors) : []
    readonly property bool isPhoneSeed: Preferences.seed === "phone" || Preferences.seed.startsWith("phone:")
    readonly property var activePhoneSeed: {
        if (!isPhoneSeed)
            return null
        const targetId = Preferences.seed.startsWith("phone:") ? Preferences.seed.slice(6) : ""
        for (let i = 0; i < phoneSeeds.length; i++) {
            const p = phoneSeeds[i]
            if (targetId !== "" && p.id === targetId && p.scheme)
                return p.scheme
        }
        for (let i = 0; i < phoneSeeds.length; i++) {
            if (phoneSeeds[i].scheme)
                return phoneSeeds[i].scheme
        }
        return null
    }
    function phonePalette(deviceId) {
        for (let i = 0; i < phoneSeeds.length; i++) {
            const p = phoneSeeds[i]
            if ((deviceId === "" || p.id === deviceId) && p.scheme)
                return p.scheme[dark ? "dark" : "light"]
        }
        return null
    }
    // "artwork", "wallpaper", "phone" / "phone:<id>", or a preset name; dynamic
    // seeds fall back to wallpaper or the default preset until their colors are known.
    readonly property string seed: isArtworkSeed
        ? (artworkSeed ? "artwork" : (wallpaperSeed ? "wallpaper" : fallbackSeed))
        : isPhoneSeed
          ? (activePhoneSeed ? Preferences.seed : (wallpaperSeed ? "wallpaper" : fallbackSeed))
          : Preferences.seed === "wallpaper"
            ? (wallpaperSeed ? "wallpaper" : fallbackSeed)
            : (bloomSeeds[Preferences.seed] ? Preferences.seed : fallbackSeed)
    readonly property var seedColors: seed === "artwork"
        ? artworkSeed
        : (seed === "phone" || seed.startsWith("phone:"))
          ? activePhoneSeed
          : (seed === "wallpaper" ? wallpaperSeed : bloomSeeds[seed])
    // Semantic colors are deliberately independent from retired style presets,
    // so existing saved Glass/Graphite preferences also receive this redesign.
    readonly property var palette: onMac ? ({
        primary: dark ? "#FFFFFF" : "#171717",
        onPrimary: dark ? "#171717" : "#FFFFFF",
        primaryContainer: dark ? "#353535" : "#EAEAEA",
        onPrimaryContainer: dark ? "#FFFFFF" : "#171717",
        secondaryContainer: dark ? "#2B2C2E" : "#EAEAEA",
        onSecondaryContainer: dark ? "#FFFFFF" : "#171717",
        surface: dark ? "#19191A" : "#F5F5F5",
        surfaceContainerLow: dark ? "#1E1E1E" : "#FFFFFF",
        surfaceContainer: dark ? "#1E1E1E" : "#FFFFFF",
        surfaceContainerHigh: dark ? "#2B2C2E" : "#F5F5F5",
        surfaceContainerHighest: dark ? "#353535" : "#EAEAEA",
        onSurface: dark ? "#FFFFFF" : "#171717",
        onSurfaceVariant: dark ? "#C2C2C2" : "#555555",
        outline: dark ? "#454545" : "#DADADA",
        outlineVariant: dark ? "#353535" : "#DADADA"
    }) : ({
        primary: dark ? "#C28AF0" : "#772AB7",
        onPrimary: dark ? "#261734" : "#FFFFFF",
        primaryContainer: dark ? "#3B2D49" : "#F1E9F8",
        onPrimaryContainer: dark ? "#D8ABF5" : "#772AB7",
        secondaryContainer: dark ? "#403246" : "#EEE6F8",
        onSecondaryContainer: dark ? "#DECAE9" : "#6A4380",
        surface: dark ? "#24212A" : "#F7F7FA",
        surfaceContainerLow: dark ? "#28242F" : "#FAF9FC",
        surfaceContainer: dark ? "#2E2A35" : "#FFFFFF",
        surfaceContainerHigh: dark ? "#37313F" : "#F1EDF5",
        surfaceContainerHighest: dark ? "#443B4E" : "#E8E2EE",
        onSurface: dark ? "#F5F1F8" : "#25222C",
        onSurfaceVariant: dark ? "#B6AEBF" : "#6A6573",
        outline: dark ? "#8C8199" : "#99929F",
        outlineVariant: dark ? "#40394A" : "#E7E4EC"
    })
    readonly property var status: ({
        warning: dark ? "#F2C477" : "#8D5A0E",
        error: dark ? "#FFA6AE" : "#C23D50",
        onError: dark ? "#4C1621" : "#FFFFFF"
    })

    // ---- Color roles ----
    readonly property color primary: palette.primary
    readonly property color primaryContent: palette.onPrimary
    readonly property color primaryContainer: palette.primaryContainer
    readonly property color primaryContainerContent: palette.onPrimaryContainer
    readonly property color secondaryContainer: palette.secondaryContainer
    readonly property color secondaryContainerContent: palette.onSecondaryContainer
    readonly property color surface: palette.surface
    readonly property color surfaceLow: palette.surfaceContainerLow
    readonly property color surfaceContainerLow: palette.surfaceContainerLow
    readonly property color surfaceContainer: palette.surfaceContainer
    readonly property color surfaceContainerHigh: palette.surfaceContainerHigh
    readonly property color surfaceContainerHighest: palette.surfaceContainerHighest
    readonly property color surfaceContent: palette.onSurface
    readonly property color surfaceContentVariant: palette.onSurfaceVariant
    readonly property color outline: palette.outline
    readonly property color outlineVariant: palette.outlineVariant
    readonly property color success: dark ? "#9BD5B4" : "#2E704D"
    readonly property color warning: status.warning
    readonly property color error: status.error
    readonly property color errorContent: status.onError
    readonly property color errorContainer: dark ? "#422732" : "#FCEBF0"
    readonly property color errorContainerContent: dark ? "#FFD8DE" : "#A22B40"

    // Neutral translucent panels reveal the existing QML ambient layer on Mac.
    readonly property color canvasColor: surface
    readonly property color panelColor: glass
        ? Qt.rgba(surfaceContainer.r, surfaceContainer.g, surfaceContainer.b, dark ? 0.84 : 0.88)
        : surfaceContainer
    readonly property color panelBorder: glass
        ? (dark ? Qt.rgba(1, 1, 1, 0.12) : Qt.rgba(0, 0, 0, 0.09))
        : outlineVariant
    readonly property color insetColor: glass
        ? (dark ? Qt.rgba(43 / 255, 44 / 255, 46 / 255, 0.84) : Qt.rgba(234 / 255, 234 / 255, 234 / 255, 0.88))
        : (onMac ? (dark ? "#2B2C2E" : "#EAEAEA") : (dark ? "#28232F" : "#F8F5FB"))
    readonly property color cardColor: panelColor
    readonly property color cardBorder: panelBorder
    readonly property color tileColor: panelColor
    readonly property color heroWash: onMac ? panelColor : (dark ? "#3B2D49" : "#F1E9F8")
    readonly property color heroWashEnd: onMac ? panelColor : (dark ? "#342A40" : "#F7F0FC")
    readonly property color heroColor: heroWash
    readonly property color heroContent: surfaceContent
    readonly property color railColor: onMac ? (dark ? "#161616" : "#F1F1F1") : (dark ? "#35104F" : "#4D137C")
    readonly property color railContent: onMac ? surfaceContent : "#FFFFFF"
    readonly property color railMuted: onMac ? surfaceContentVariant : (dark ? "#D8C7E5" : "#DDCCEC")
    readonly property color railSelected: onMac ? (dark ? "#353535" : "#DADADA") : Qt.rgba(1, 1, 1, 0.16)
    readonly property color indicator: primaryContainer
    readonly property color indicatorContent: primaryContainerContent
    readonly property color actionColor: primary
    readonly property color actionContent: primaryContent
    readonly property color focusColor: primary
    readonly property color glow: "transparent"
    readonly property color buttonShadow: "transparent"
    readonly property color scrim: onMac ? Qt.rgba(0, 0, 0, dark ? 0.64 : 0.30) : Qt.rgba(0.04, 0.08, 0.15, dark ? 0.64 : 0.30)
    readonly property int controlRadius: 6
    readonly property int cardRadius: 10
    readonly property int heroRadius: 14
    readonly property int spacingXs: 4
    readonly property int spacingSm: 8
    readonly property int spacingMd: 12
    readonly property int spacingLg: 16
    readonly property int spacingXl: 24
    readonly property int spacingXxl: 32
    readonly property int sidebarWidth: 224
    readonly property int compactRailWidth: 72

    // ---- Shape ----
    readonly property var shape: ({ xs: 4, sm: 6, md: 8, lg: 10, xl: 14, full: 6 })
    readonly property int radiusXs: shape.xs
    readonly property int radiusSm: shape.sm
    readonly property int radiusMd: shape.md
    readonly property int radiusLg: shape.lg
    readonly property int radiusXl: shape.xl
    function pill(height) { return Math.min(controlRadius, height / 2) }

    // ---- Type ----
    // Bundled with the app (assets/fonts).
    readonly property string fontUi: "Figtree"
    readonly property string fontDisplay: "Figtree"
    readonly property string fontMono: "Space Mono"
    readonly property var typography: Tokens.data.typography
    readonly property var labelStyle: graphite ? Tokens.data.themes.graphite.label
        : (hive ? Tokens.data.themes.hive.label : Tokens.data.themes.bloom.label)

    // ---- Layout and spacing ----
    readonly property var layout: Tokens.data.layout.desktop
    readonly property int railWidth: Qt.platform.os === "osx" || Qt.platform.os === "macos" ? compactRailWidth : layout.railWidth
    readonly property int topBarHeight: Qt.platform.os === "osx" || Qt.platform.os === "macos" ? 44 : layout.topBarHeight
    readonly property int contentPadding: spacingXl
    readonly property int gutter: layout.gutter
    function space(step) { return Tokens.data.spacing.scale[step] }

    // Focus rings show for keyboard focus only, not after a click (TapHandler
    // doesn't take activeFocus on click; activeFocus on a control comes from Tab).
    function focusVisible(item) {
        return item !== null && item !== undefined && item.activeFocus
            && (item.focusReason === undefined
                || item.focusReason === Qt.TabFocusReason
                || item.focusReason === Qt.BacktabFocusReason)
    }

    // ---- Motion ----
    // QML springs take a strength and a damping ratio; these are tuned to
    // match the token springs (gentle / standard / snappy).
    readonly property real springGentle: 2.4
    readonly property real dampingGentle: 0.36
    readonly property real springStandard: 3.6
    readonly property real dampingStandard: 0.32
    readonly property real springSnappy: 5.6
    readonly property real dampingSnappy: 0.34
    readonly property int fadeFast: Tokens.data.motion.fade.fast
    readonly property int fadeNormal: Tokens.data.motion.fade.normal
    readonly property real hoverLift: reduceMotion ? 0 : Tokens.data.motion.hoverLift
    readonly property real pressScale: reduceMotion ? 1 : 0.96
}

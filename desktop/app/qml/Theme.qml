// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick
import app.nectarlink

// Waypair's native violet palette is shared across all screens. Legacy style
// preferences remain stored for compatibility; light/dark follows colorMode.
QtObject {
    id: theme

    readonly property bool hive: false
    readonly property bool graphite: false
    readonly property bool glass: false
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
    readonly property color combGlow: dark ? Qt.rgba(1.0, 0.62, 0.18, comb.glowAlphaDark)
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
    readonly property var palette: ({
        primary: dark ? "#C49BFF" : "#8A2ED4",
        onPrimary: dark ? "#281542" : "#FFFFFF",
        primaryContainer: dark ? "#45305F" : "#EFE5FF",
        onPrimaryContainer: dark ? "#E9D7FF" : "#6626A6",
        secondaryContainer: dark ? "#3A2C50" : "#F0EAF9",
        onSecondaryContainer: dark ? "#E1CEF9" : "#65427E",
        surface: dark ? "#171426" : "#EEF0F8",
        surfaceContainerLow: dark ? "#1C172D" : "#F6F6FC",
        surfaceContainer: dark ? "#231E38" : "#FFFFFF",
        surfaceContainerHigh: dark ? "#2E2745" : "#F0EEF8",
        surfaceContainerHighest: dark ? "#3E3456" : "#E3DFF0",
        onSurface: dark ? "#EEEBFA" : "#211740",
        onSurfaceVariant: dark ? "#B9AECC" : "#726984",
        outline: dark ? "#807091" : "#A69BB8",
        outlineVariant: dark ? "#393048" : "#E4DEED"
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
    readonly property color success: primary
    readonly property color warning: status.warning
    readonly property color error: status.error
    readonly property color errorContent: status.onError
    readonly property color errorContainer: dark ? "#422732" : "#FCEBF0"
    readonly property color errorContainerContent: dark ? "#FFD8DE" : "#A22B40"

    // Opaque panels and quiet insets keep the dashboard readable in both modes.
    readonly property color canvasColor: surface
    readonly property color panelColor: surfaceContainer
    readonly property color panelBorder: outlineVariant
    readonly property color insetColor: dark ? "#1F1A31" : "#F6F4FA"
    readonly property color cardColor: panelColor
    readonly property color cardBorder: panelBorder
    readonly property color tileColor: panelColor
    readonly property color heroWash: dark ? "#302445" : "#F2EAFE"
    readonly property color heroWashEnd: dark ? "#352746" : "#F5EEFC"
    readonly property color heroColor: heroWash
    readonly property color heroContent: surfaceContent
    readonly property color railColor: dark ? "#351077" : "#5813D7"
    readonly property color railContent: "#F8F3FF"
    readonly property color railMuted: "#CEB9EE"
    readonly property color railSelected: Qt.rgba(1, 1, 1, 0.16)
    readonly property color indicator: primaryContainer
    readonly property color indicatorContent: primaryContainerContent
    readonly property color actionColor: dark ? "#BE93FF" : "#9A42F4"
    readonly property color actionContent: primaryContent
    readonly property color focusColor: primary
    readonly property color glow: "transparent"
    readonly property color buttonShadow: "transparent"
    readonly property color scrim: Qt.rgba(0.04, 0.08, 0.15, dark ? 0.64 : 0.30)
    readonly property int controlRadius: 8
    readonly property int cardRadius: 14
    readonly property int heroRadius: 20
    readonly property int spacingXs: 4
    readonly property int spacingSm: 8
    readonly property int spacingMd: 12
    readonly property int spacingLg: 16
    readonly property int spacingXl: 24
    readonly property int spacingXxl: 32
    readonly property int sidebarWidth: 224
    readonly property int compactRailWidth: 80

    // ---- Shape ----
    readonly property var shape: ({ xs: 4, sm: 6, md: 10, lg: 14, xl: 20, full: 8 })
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
    readonly property int railWidth: layout.railWidth
    readonly property int topBarHeight: layout.topBarHeight
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
    readonly property real pressScale: reduceMotion ? 1 : Tokens.data.motion.pressScale
}

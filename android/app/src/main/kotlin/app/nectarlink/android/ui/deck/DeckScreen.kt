// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.deck

import app.nectarlink.android.ui.theme.linkSuccessColor
import androidx.compose.ui.graphics.luminance
import app.nectarlink.android.ui.theme.linkIconColor
import app.nectarlink.android.ui.theme.LinkIcon
import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import android.view.HapticFeedbackConstants
import android.view.WindowManager
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import app.nectarlink.android.ui.theme.LinkFilledTonalButton as FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import app.nectarlink.android.ui.theme.LinkOutlinedButton as OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import app.nectarlink.android.ui.theme.LinkTextButton as TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.home.PcAudioOutputsDialog
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.WaypairDesign
import app.nectarlink.core.DeckLayout
import app.nectarlink.core.DeckPage
import app.nectarlink.core.DeckState
import app.nectarlink.core.DeckTile
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun DeckScreen(
    device: Device,
    core: Core,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val view = LocalView.current
    val context = LocalContext.current
    val configuration = LocalConfiguration.current
    val isLandscape = configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
    val scope = rememberCoroutineScope()

    var fullScreen by rememberSaveable { mutableStateOf(false) }
    var access by remember(device.id) { mutableStateOf(Core.RemoteAccess.Allowed) }
    var commandsDenied by remember(device.id) { mutableStateOf(false) }

    // Keep the phone's screen on while Deck is open, and restore system bars on exit.
    DisposableEffect(view, device.id) {
        val window = (context as? Activity)?.window
        val prevKeepOn = view.keepScreenOn
        view.keepScreenOn = true
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            view.keepScreenOn = prevKeepOn
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            if (window != null) {
                WindowCompat.getInsetsController(window, view)
                    .show(WindowInsetsCompat.Type.systemBars())
            }
        }
    }

    LaunchedEffect(fullScreen, view) {
        val window = (context as? Activity)?.window ?: return@LaunchedEffect
        val controller = WindowCompat.getInsetsController(window, view)
        if (fullScreen) {
            controller.systemBarsBehavior =
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    // Check remote_input access on entry and poll while denied so the banner clears
    // as soon as the user clicks Allow on the PC.
    LaunchedEffect(device.id, device.online) {
        if (!device.online) {
            access = Core.RemoteAccess.Offline
            return@LaunchedEffect
        }
        access = core.remoteCheck(device.id)
        while (access == Core.RemoteAccess.Denied && device.online) {
            delay(1_500)
            access = core.remoteCheck(device.id)
        }
    }

    val reducedMotion = LocalReducedMotion.current
    val layout = device.deckLayout ?: remember(context) { fallbackDefaultLayout(context) }
    val liveState = device.deckState ?: DeckState(
        playing = false,
        volume = 50u,
        muted = false,
        micMuted = false,
        outputDevices = emptyList(),
    )
    val pages = layout.pages.ifEmpty { fallbackDefaultLayout(context).pages }
    val pagerState = rememberPagerState(pageCount = { pages.size })

    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(
                horizontal = 16.dp,
                vertical = if (fullScreen || isLandscape) 8.dp else 12.dp,
            ),
        verticalArrangement = Arrangement.spacedBy(if (isLandscape) 8.dp else 12.dp),
    ) {
        // Header row: Back, PC name + status dot, Full-screen toggle
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(
                    painter = painterResource(R.drawable.ic_back),
                    contentDescription = stringResource(R.string.action_back),
                )
            }
            LinkIcon(R.drawable.wp_deck_color, contentDescription = null, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        device.name,
                        style = if (isLandscape) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.width(8.dp))
                    Surface(
                        shape = CircleShape,
                        color = if (device.online) linkSuccessColor(MaterialTheme.colorScheme.onSurface) else MaterialTheme.colorScheme.outline,
                        modifier = Modifier.size(8.dp),
                    ) {}
                }
                if (!fullScreen && !isLandscape) {
                    Text(
                        stringResource(R.string.deck_hint, device.name),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            OutlinedButton(
                onClick = { fullScreen = !fullScreen },
                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(
                    stringResource(if (fullScreen) R.string.deck_exit_fullscreen else R.string.deck_fullscreen),
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        }

        // Compact PC audio header bar (volume, mute, and active output device)
        if (!fullScreen && device.online) {
            DeckAudioHeaderBar(
                pcName = device.name,
                liveState = liveState,
                onSetPcAudio = { vol, muted -> core.setPcAudio(device.id, vol, muted) },
            )
        }

        // Status / permission banners
        if (!device.online) {
            DeckBanner(
                title = stringResource(R.string.remote_offline_text, device.name),
                text = null,
            )
        } else if (access == Core.RemoteAccess.Denied) {
            DeckBanner(
                title = stringResource(R.string.remote_denied_title, device.name),
                text = stringResource(R.string.remote_denied_text, device.name),
            )
        } else if (commandsDenied) {
            DeckBanner(
                title = stringResource(R.string.deck_commands_denied_title, device.name),
                text = stringResource(R.string.deck_commands_denied_text, device.name),
                onDismiss = { commandsDenied = false },
            )
        }

        // Page tabs when multiple pages exist
        if (pages.size > 1) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                pages.forEachIndexed { index, page ->
                    FilterChip(
                        selected = pagerState.currentPage == index,
                        onClick = {
                            scope.launch {
                                if (reducedMotion) {
                                    pagerState.scrollToPage(index)
                                } else {
                                    pagerState.animateScrollToPage(index)
                                }
                            }
                        },
                        label = {
                            Text(
                                page.name,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        },
                    )
                }
            }
        }

        // Swipable pages of tactile Deck tiles
        HorizontalPager(
            state = pagerState,
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
        ) { pageIndex ->
            val page = pages.getOrNull(pageIndex) ?: pages.first()
            if (page.tiles.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        stringResource(R.string.deck_empty_page, device.name),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialResource.onSurfaceVariant(),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(24.dp),
                    )
                }
            } else {
                val largeFont = LocalConfiguration.current.fontScale >= 1.2f
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(minSize = if (largeFont) 144.dp else 112.dp),
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                    verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                ) {
                    items(page.tiles, key = { it.id }) { tile ->
                        DeckTileButton(
                            tile = tile,
                            liveState = liveState,
                            compact = isLandscape,
                            enabled = device.online,
                            onPress = {
                                view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
                                scope.launch {
                                    when (core.deckPress(device.id, tile.id)) {
                                        Core.DeckPressResult.Ok -> {
                                            access = Core.RemoteAccess.Allowed
                                            if (tile.kind == "run_command") commandsDenied = false
                                        }
                                        Core.DeckPressResult.Denied -> {
                                            val chk = core.remoteCheck(device.id)
                                            access = chk
                                            if (chk == Core.RemoteAccess.Allowed && tile.kind == "run_command") {
                                                commandsDenied = true
                                            }
                                        }
                                        Core.DeckPressResult.Offline -> {
                                            access = Core.RemoteAccess.Offline
                                        }
                                        else -> {}
                                    }
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

private object MaterialResource {
    @Composable
    fun onSurfaceVariant(): Color = MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
private fun DeckAudioHeaderBar(
    pcName: String,
    liveState: DeckState,
    onSetPcAudio: (volume: Int?, muted: Boolean?) -> Unit,
) {
    var draggingVolume by remember { mutableStateOf<Float?>(null) }
    var lastSentVolume by remember { mutableStateOf(liveState.volume.toInt()) }
    if (draggingVolume == null) {
        lastSentVolume = liveState.volume.toInt()
    }
    var showOutputs by remember { mutableStateOf(false) }
    val shownVolume = draggingVolume?.roundToInt() ?: liveState.volume.toInt()
    val defaultOutput = liveState.outputDevices.firstOrNull { it.isDefault } ?: liveState.outputDevices.firstOrNull()
    val volumeLabel = stringResource(R.string.pc_audio_volume)

    Surface(
        shape = RoundedCornerShape(WaypairDesign.cardRadius),
        color = WaypairDesign.panelColor(),
        border = BorderStroke(1.dp, WaypairDesign.panelBorder()),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(WaypairDesign.spacingLg),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilledTonalButton(
                    onClick = { onSetPcAudio(null, !liveState.muted) },
                    modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                    contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                ) {
                    Text(
                        stringResource(if (liveState.muted) R.string.pc_audio_unmute else R.string.pc_audio_mute),
                        style = MaterialTheme.typography.labelMedium,
                        maxLines = 1,
                    )
                }
                Slider(
                    value = (draggingVolume ?: liveState.volume.toFloat()).coerceIn(0f, 100f),
                    onValueChange = { v ->
                        draggingVolume = v
                        val rounded = v.roundToInt()
                        if (rounded != lastSentVolume) {
                            lastSentVolume = rounded
                            onSetPcAudio(rounded, null)
                        }
                    },
                    onValueChangeFinished = { draggingVolume = null },
                    valueRange = 0f..100f,
                    modifier = Modifier
                        .weight(1f)
                        .semantics { contentDescription = volumeLabel },
                )
                Text(
                    text = if (liveState.muted) {
                        stringResource(R.string.pc_audio_muted)
                    } else {
                        stringResource(R.string.deck_volume_percent, shownVolume)
                    },
                    style = MaterialTheme.typography.labelMedium,
                )
            }
            if (defaultOutput != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = WaypairDesign.touchTarget)
                        .clickable(role = Role.Button) { showOutputs = true }
                        .padding(horizontal = 4.dp, vertical = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = stringResource(R.string.pc_audio_output, defaultOutput.name),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (liveState.outputDevices.size > 1) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = "${liveState.outputDevices.size}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                        )
                    }
                }
            }
        }
    }

    if (showOutputs && liveState.outputDevices.isNotEmpty()) {
        PcAudioOutputsDialog(
            pcName = pcName,
            devices = liveState.outputDevices,
            onDismiss = { showOutputs = false },
        )
    }
}

@Composable
private fun DeckBanner(
    title: String,
    text: String?,
    onDismiss: (() -> Unit)? = null,
) {
    Surface(
        shape = RoundedCornerShape(WaypairDesign.cardRadius),
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                if (text != null) {
                    Spacer(Modifier.height(2.dp))
                    Text(text, style = MaterialTheme.typography.bodyMedium)
                }
            }
            if (onDismiss != null) {
                val dismissLabel = stringResource(R.string.action_dismiss)
                TextButton(
                    onClick = onDismiss,
                    modifier = Modifier.semantics { contentDescription = dismissLabel },
                ) {
                    Icon(painterResource(R.drawable.wp_close), contentDescription = null, modifier = Modifier.size(20.dp))
                }
            }
        }
    }
}

private data class TilePalette(
    val bg: Color,
    val border: Color,
    val accent: Color,
    val fg: Color,
    val iconWell: Color,
)

@Composable
private fun tilePalette(
    @Suppress("UNUSED_PARAMETER") colorName: String,
    highlighted: Boolean,
): TilePalette {
    val scheme = MaterialTheme.colorScheme
    return if (highlighted) {
        TilePalette(
            bg = scheme.primaryContainer,
            border = scheme.primary,
            accent = scheme.primary,
            fg = scheme.onPrimaryContainer,
            iconWell = scheme.surfaceContainerHigh,
        )
    } else {
        TilePalette(
            bg = scheme.surfaceContainerHigh,
            border = scheme.outlineVariant,
            accent = scheme.primary,
            fg = scheme.onSurface,
            iconWell = scheme.secondaryContainer,
        )
    }
}

@Composable
private fun liveBadgeText(kind: String, state: DeckState): String? = when (kind) {
    "media_play_pause" -> stringResource(if (state.playing) R.string.deck_playing else R.string.deck_paused)
    "volume_mute" ->
        if (state.muted) stringResource(R.string.deck_muted)
        else if (state.volume > 0u) stringResource(R.string.deck_volume_percent, state.volume.toInt())
        else null
    "mic_mute" -> when (state.micMuted) {
        true -> stringResource(R.string.deck_muted)
        false -> stringResource(R.string.deck_live)
        null -> null
    }
    else -> null
}

private fun isLiveHighlighted(kind: String, state: DeckState): Boolean = when (kind) {
    "media_play_pause" -> state.playing
    "volume_mute" -> state.muted
    "mic_mute" -> state.micMuted == true
    else -> false
}

private fun effectiveIcon(tile: DeckTile, state: DeckState): String = when {
    tile.kind == "media_play_pause" && state.playing -> "pause"
    tile.kind == "mic_mute" && state.micMuted == true -> "mic_off"
    tile.kind == "volume_mute" && state.muted -> "volume_off"
    else -> tile.icon
}

@Composable
private fun DeckTileButton(
    tile: DeckTile,
    liveState: DeckState,
    compact: Boolean,
    enabled: Boolean,
    onPress: () -> Unit,
) {
    val reducedMotion = LocalReducedMotion.current
    val badge = liveBadgeText(tile.kind, liveState)
    val highlighted = isLiveHighlighted(tile.kind, liveState)
    val palette = tilePalette(tile.color, highlighted)
    val iconName = effectiveIcon(tile, liveState)

    var pressed by remember { mutableStateOf(false) }
    val scale by animateFloatAsState(
        targetValue = if (pressed && !reducedMotion) 0.96f else 1f,
        animationSpec = if (reducedMotion) snap() else spring(stiffness = Spring.StiffnessHigh),
        label = "tileScale",
    )

    val a11yDescription = if (badge != null) {
        stringResource(R.string.deck_tile_with_badge, tile.label, badge)
    } else {
        tile.label
    }
    val fontScale = LocalConfiguration.current.fontScale.coerceIn(1f, 1.35f)
    val tileHeight = ((if (compact) 98 else 118) * fontScale).dp

    Surface(
        shape = RoundedCornerShape(WaypairDesign.cardRadius),
        color = palette.bg.copy(alpha = if (enabled) 1f else 0.55f),
        contentColor = palette.fg,
        border = BorderStroke(
            width = if (highlighted) 2.dp else 1.dp,
            color = if (highlighted) palette.accent else palette.border,
        ),
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = tileHeight)
            .graphicsLayer {
                scaleX = scale
                scaleY = scale
            }
            .semantics(mergeDescendants = true) {
                role = Role.Button
                contentDescription = a11yDescription
                if (enabled) {
                    onClick(label = a11yDescription) {
                        onPress()
                        true
                    }
                }
            }
            .pointerInput(enabled, tile.id) {
                if (!enabled) return@pointerInput
                detectTapGestures(
                    onPress = {
                        pressed = true
                        try {
                            if (tryAwaitRelease()) {
                                onPress()
                            }
                        } finally {
                            pressed = false
                        }
                    },
                )
            },
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(WaypairDesign.spacingMd),
            verticalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top,
            ) {
                Box(
                    modifier = Modifier
                        .size(if (compact) 34.dp else 36.dp)
                        .background(palette.iconWell, MaterialTheme.shapes.medium),
                    contentAlignment = Alignment.Center,
                ) {
                    DeckIcon(
                        name = iconName,
                        tint = if (enabled) deckGlyphColor(tile.color, palette.fg) else palette.fg.copy(alpha = 0.42f),
                        modifier = Modifier.size(if (compact) 18.dp else 20.dp),
                    )
                }

                if (badge != null) {
                    Spacer(Modifier.width(6.dp))
                    Surface(
                        shape = CircleShape,
                        color = if (highlighted) palette.accent else palette.iconWell,
                        contentColor = if (highlighted) MaterialTheme.colorScheme.onPrimary else palette.fg,
                    ) {
                        Text(
                            text = badge,
                            style = MaterialTheme.typography.labelSmall,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            softWrap = false,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                        )
                    }
                }
            }

            Text(
                text = tile.label,
                style = if (compact) MaterialTheme.typography.labelLarge else MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                color = palette.fg,
            )
        }
    }
}

@Composable
private fun DeckIcon(
    name: String,
    tint: Color,
    modifier: Modifier = Modifier,
) {
    val iconRes = when (name) {
        "play" -> R.drawable.ic_play
        "play_pause" -> R.drawable.ic_play
        "pause" -> R.drawable.ic_pause
        "skip_next" -> R.drawable.ic_skip_next
        "skip_previous" -> R.drawable.ic_skip_previous
        "volume_up" -> R.drawable.wp_oneui_volume_up
        "volume_down" -> R.drawable.wp_oneui_volume_down
        "volume_off" -> R.drawable.ic_sound_off
        "mic" -> R.drawable.wp_mic_color
        "mic_off" -> R.drawable.wp_oneui_mic_off
        "lock" -> R.drawable.wp_oneui_lock_outline
        "desktop" -> R.drawable.wp_monitor
        "switch_window" -> R.drawable.wp_oneui_system_alert_window
        "windows" -> R.drawable.wp_oneui_system_alert_window
        "screenshot" -> R.drawable.wp_camera_color
        "camera" -> R.drawable.wp_camera_color
        "keyboard" -> R.drawable.wp_oneui_keyboard
        "globe" -> R.drawable.wp_globe
        "text" -> R.drawable.wp_oneui_font_size
        "app" -> R.drawable.wp_oneui_apps_outline
        "rocket" -> R.drawable.wp_oneui_apps_outline
        "terminal" -> R.drawable.wp_oneui_bracket
        "spark" -> R.drawable.wp_oneui_lightning_outline
        "bolt" -> R.drawable.wp_oneui_lightning_outline
        "music" -> R.drawable.ic_music
        "video" -> R.drawable.wp_oneui_video_call_outline
        "folder" -> R.drawable.wp_folder_color
        "star" -> R.drawable.wp_oneui_star_outline
        "bell" -> R.drawable.wp_bell_color
        "power" -> R.drawable.wp_oneui_power
        else -> R.drawable.wp_oneui_power
    }
    Icon(painterResource(iconRes), contentDescription = null, modifier = modifier, tint = tint)
}

private fun fallbackDefaultLayout(context: Context): DeckLayout = DeckLayout(
    pages = listOf(
        DeckPage(
            id = "main",
            name = context.getString(R.string.deck_default_page_main),
            tiles = listOf(
                DeckTile("play_pause", context.getString(R.string.deck_default_play_pause), "play", "amber", "media_play_pause"),
                DeckTile("prev_track", context.getString(R.string.deck_default_previous), "skip_previous", "slate", "media_previous"),
                DeckTile("next_track", context.getString(R.string.deck_default_next), "skip_next", "slate", "media_next"),
                DeckTile("vol_down", context.getString(R.string.deck_default_volume_down), "volume_down", "teal", "volume_down"),
                DeckTile("vol_up", context.getString(R.string.deck_default_volume_up), "volume_up", "teal", "volume_up"),
                DeckTile("vol_mute", context.getString(R.string.deck_default_volume_mute), "volume_off", "teal", "volume_mute"),
                DeckTile("mic_mute", context.getString(R.string.deck_default_mic_mute), "mic", "coral", "mic_mute"),
                DeckTile("show_desktop", context.getString(R.string.deck_default_show_desktop), "desktop", "blue", "show_desktop"),
                DeckTile("switch_window", context.getString(R.string.deck_default_switch_window), "switch_window", "blue", "switch_window"),
                DeckTile("screenshot", context.getString(R.string.deck_default_screenshot), "screenshot", "violet", "screenshot"),
                DeckTile("lock_pc", context.getString(R.string.deck_default_lock_pc), "lock", "red", "lock_pc"),
            ),
        ),
    ),
)

private fun deckGlyphColor(name: String, fallback: Color): Color = when (name.lowercase()) {
    "amber" -> linkIconColor(R.drawable.wp_bell_color, fallback)
    "teal" -> linkIconColor(R.drawable.wp_folder_color, fallback)
    "blue" -> linkIconColor(R.drawable.wp_monitor, fallback)
    "violet" -> linkIconColor(R.drawable.wp_camera_color, fallback)
    "slate" -> linkIconColor(R.drawable.wp_settings, fallback)
    "coral" -> if (fallback.luminance() >= 0.5f) Color(0xFFF5A08B) else Color(0xFFAD503C)
    "red" -> if (fallback.luminance() >= 0.5f) Color(0xFFFF9C9C) else Color(0xFFBA3A45)
    else -> fallback
}

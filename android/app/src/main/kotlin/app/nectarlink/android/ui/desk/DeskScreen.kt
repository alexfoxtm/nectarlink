// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.desk

import android.app.Activity
import android.view.WindowManager
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ButtonDefaults
import app.nectarlink.android.ui.theme.LinkFilledTonalButton as FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import app.nectarlink.android.ui.theme.LinkOutlinedButton as OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.home.canControlPc
import app.nectarlink.android.ui.home.canOpenDeck
import app.nectarlink.android.ui.remote.RemoteMode
import app.nectarlink.android.ui.theme.WaypairDesign
import app.nectarlink.core.Link
import app.nectarlink.core.MediaAction
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Full-screen ambient Desk Mode dock (`docs/PLAN.md` §3.13 / §8.6 3.4):
 * keeps the display awake with subtle 1-minute OLED burn-in pixel shift,
 * large tabular clock & date, live PC song title/artist/artwork & audio controls,
 * and a balanced 2×2 quick-action dock.
 */
@Composable
fun DeskScreen(
    device: Device,
    core: Core,
    onOpenRemote: (RemoteMode) -> Unit,
    onOpenDeck: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    DisposableEffect(Unit) {
        val window = (context as? Activity)?.window
        window?.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val controller = window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller?.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    var minuteBucket by remember { mutableLongStateOf(System.currentTimeMillis() / 60_000L) }
    LaunchedEffect(Unit) {
        while (true) {
            val now = System.currentTimeMillis()
            minuteBucket = now / 60_000L
            delay((60_000L - (now % 60_000L)).coerceIn(1_000L, 60_000L))
        }
    }

    // Subtle 1-minute OLED burn-in protection pixel shift (±8 px X, ±6 px Y) for the ambient clock/card.
    val minutePhase = (minuteBucket % 12L).toDouble() * (Math.PI / 6.0)
    val shiftX = (sin(minutePhase) * 8.0).roundToInt()
    val shiftY = (cos(minutePhase) * 6.0).roundToInt()

    val deckState = device.deckState
    val pcMediaSummary by core.pcMedia.summaryFlow.collectAsStateWithLifecycle()
    val activeSummary = pcMediaSummary?.takeIf { it.pcId == device.id } ?: pcMediaSummary
    val artBitmap = remember(activeSummary?.artwork) { activeSummary?.artwork?.asImageBitmap() }

    var draggingVolume by remember { mutableStateOf<Float?>(null) }
    var lastSentVolInt by remember { mutableIntStateOf(-1) }
    val scope = rememberCoroutineScope()

    Surface(
        color = MaterialTheme.colorScheme.surface,
        contentColor = MaterialTheme.colorScheme.onSurface,
        modifier = modifier.fillMaxSize(),
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(WaypairDesign.spacingLg),
            verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingLg),
        ) {
            // Top bar: PC connection status + Exit button
            Row(
                modifier = Modifier
                    .fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Surface(
                    shape = RoundedCornerShape(WaypairDesign.controlRadius),
                    modifier = Modifier.weight(1f),
                    color = if (device.online) MaterialTheme.colorScheme.surfaceContainerHigh else MaterialTheme.colorScheme.surfaceContainerLow,
                    border = BorderStroke(
                        1.dp,
                        if (device.online) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.outlineVariant,
                    ),
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        val dotColor = if (device.online) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                        Spacer(
                            modifier = Modifier
                                .size(8.dp)
                                .drawBehind { drawCircle(dotColor) },
                        )
                        val linkDetail = when (val l = device.link) {
                            is Link.Online -> "${device.name} · ${l.rttMs} ms"
                            else -> "${device.name} · Offline"
                        }
                        Text(
                            text = linkDetail,
                            style = MaterialTheme.typography.labelLarge,
                            color = if (device.online) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                Spacer(Modifier.width(WaypairDesign.spacingMd))
                OutlinedButton(
                    onClick = onBack,
                    modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                ) {
                    Text(stringResource(R.string.desk_exit), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }

            // Center: Ambient Clock + Date + PC Media / Volume Card (with OLED burn-in drift)
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .verticalScroll(rememberScrollState())
                    .offset { IntOffset(shiftX, shiftY) },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingLg, Alignment.CenterVertically),
            ) {
                DeskClockDisplay()

                Spacer(Modifier.height(2.dp))

                // PC Media & Audio Dock Card
                if (device.online && (deckState != null || activeSummary != null)) {
                    val isPlaying = activeSummary?.playing ?: (deckState?.playing == true)
                    val outName = deckState?.outputDevices?.firstOrNull { it.isDefault }?.name
                        ?: deckState?.outputDevices?.firstOrNull()?.name
                        ?: device.name
                    val songTitle = activeSummary?.title?.takeIf { it.isNotBlank() }
                        ?: if (isPlaying) stringResource(R.string.desk_media_playing) else stringResource(R.string.desk_media_audio)
                    val subtitleText = buildString {
                        val sub = activeSummary?.subtitle?.trim().orEmpty()
                        if (sub.isNotEmpty()) {
                            append(sub)
                            if (outName.isNotBlank() && !sub.contains(outName, ignoreCase = true)) {
                                append(" · ")
                                append(outName)
                            }
                        } else {
                            append(outName)
                        }
                    }

                    Surface(
                        shape = RoundedCornerShape(WaypairDesign.cardRadius),
                        color = MaterialTheme.colorScheme.surfaceContainerLow,
                        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                            verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                        ) {
                            Column(
                                modifier = Modifier.fillMaxWidth(),
                                verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                            ) {
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    // Album art or music badge
                                    if (artBitmap != null) {
                                        Image(
                                            bitmap = artBitmap,
                                            contentDescription = null,
                                            contentScale = ContentScale.Crop,
                                            modifier = Modifier
                                                .size(48.dp)
                                                .clip(RoundedCornerShape(12.dp)),
                                        )
                                    } else {
                                        Box(
                                            modifier = Modifier
                                                .size(48.dp)
                                                .clip(RoundedCornerShape(12.dp))
                                                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.16f)),
                                            contentAlignment = Alignment.Center,
                                        ) {
                                            Icon(
                                                painter = painterResource(R.drawable.ic_music),
                                                contentDescription = null,
                                                tint = MaterialTheme.colorScheme.primary,
                                                modifier = Modifier.size(20.dp),
                                            )
                                        }
                                    }

                                    Column(
                                        modifier = Modifier.weight(1f),
                                        verticalArrangement = Arrangement.spacedBy(2.dp),
                                    ) {
                                        Text(
                                            text = songTitle,
                                            style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold),
                                            color = if (activeSummary != null) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.primary,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                        Text(
                                            text = subtitleText,
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    }

                                }
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingSm, Alignment.End),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.pressed(MediaAction.PREVIOUS)
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_previous") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                                            contentColor = MaterialTheme.colorScheme.onSurface,
                                        ),
                                        modifier = Modifier.size(WaypairDesign.touchTarget),
                                    ) {
                                        Icon(
                                            painterResource(R.drawable.ic_skip_previous),
                                            contentDescription = stringResource(R.string.media_previous),
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.pressed(
                                                    if (isPlaying) MediaAction.PAUSE else MediaAction.PLAY,
                                                )
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_play_pause") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = MaterialTheme.colorScheme.primaryContainer,
                                            contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                                        ),
                                        modifier = Modifier.size(WaypairDesign.touchTarget),
                                    ) {
                                        Icon(
                                            painterResource(
                                                if (isPlaying) R.drawable.ic_pause else R.drawable.ic_play,
                                            ),
                                            contentDescription = stringResource(R.string.deck_default_play_pause),
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                    FilledTonalIconButton(
                                        onClick = {
                                            if (activeSummary != null) {
                                                core.pcMedia.pressed(MediaAction.NEXT)
                                            } else {
                                                scope.launch { core.deckPress(device.id, "media_next") }
                                            }
                                        },
                                        colors = IconButtonDefaults.filledTonalIconButtonColors(
                                            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                                            contentColor = MaterialTheme.colorScheme.onSurface,
                                        ),
                                        modifier = Modifier.size(WaypairDesign.touchTarget),
                                    ) {
                                        Icon(
                                            painterResource(R.drawable.ic_skip_next),
                                            contentDescription = stringResource(R.string.media_next),
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                }
                            }

                            // Volume slider row
                            if (deckState != null) {
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    IconButton(
                                        onClick = { core.setPcAudio(device.id, null, !deckState.muted) },
                                        modifier = Modifier.size(WaypairDesign.touchTarget),
                                    ) {
                                        Icon(
                                            painterResource(
                                                if (deckState.muted) R.drawable.ic_sound_off else R.drawable.ic_speaker,
                                            ),
                                            contentDescription = null,
                                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                            modifier = Modifier.size(18.dp),
                                        )
                                    }
                                    val shownVol = (draggingVolume ?: deckState.volume.toFloat()).coerceIn(0f, 100f)
                                    Slider(
                                        value = shownVol,
                                        onValueChange = { v ->
                                            draggingVolume = v
                                            val rounded = v.roundToInt()
                                            if (rounded != lastSentVolInt) {
                                                lastSentVolInt = rounded
                                                core.setPcAudio(device.id, rounded, null)
                                            }
                                        },
                                        onValueChangeFinished = {
                                            draggingVolume = null
                                            lastSentVolInt = -1
                                        },
                                        valueRange = 0f..100f,
                                        colors = SliderDefaults.colors(
                                            thumbColor = MaterialTheme.colorScheme.primary,
                                            activeTrackColor = MaterialTheme.colorScheme.primary,
                                            inactiveTrackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                                        ),
                                        modifier = Modifier.weight(1f),
                                    )
                                    Text(
                                        text = if (deckState.muted) "Muted" else "${shownVol.roundToInt()}%",
                                        style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        modifier = Modifier.width(44.dp),
                                    )
                                }
                            }
                        }
                    }
                } else if (!device.online && device.canWake) {
                    FilledTonalButton(onClick = { core.wake(device.id) }) {
                        Text(stringResource(R.string.action_wake_pc))
                    }
                }
            }

            // Negotiated capability gates, same policy as the Home PC card:
            // without `input.inject` / `deck.actions` the desktop answers
            // Unsupported (Mac withholds both), so the entries stay greyed
            // with the existing unsupported copy. Clock, media card, wake,
            // and Lock PC stay as before (local render / media / `pc.power`,
            // which Mac advertises).
            val unsupportedReason = stringResource(R.string.error_unsupported)
            val controlPcEnabled = device.canControlPc()
            val deckEnabled = device.canOpenDeck()
            val controlGating =
                if (device.online && !controlPcEnabled) {
                    Modifier.semantics { stateDescription = unsupportedReason }
                } else {
                    Modifier
                }
            val deckGating =
                if (device.online && !deckEnabled) {
                    Modifier.semantics { stateDescription = unsupportedReason }
                } else {
                    Modifier
                }

            // Bottom Dock Bar: Symmetric 2×2 Grid of Equal-Width Quick Action Pills
            Column(
                modifier = Modifier
                    .fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                ) {
                    DeskActionButton(
                        label = stringResource(R.string.desk_pc_screen),
                        enabled = controlPcEnabled,
                        onClick = { onOpenRemote(RemoteMode.Screen) },
                        modifier = Modifier.weight(1f).then(controlGating),
                    )
                    DeskActionButton(
                        label = stringResource(R.string.action_remote_pc),
                        enabled = controlPcEnabled,
                        onClick = { onOpenRemote(RemoteMode.Touchpad) },
                        modifier = Modifier.weight(1f).then(controlGating),
                    )
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
                ) {
                    DeskActionButton(
                        label = stringResource(R.string.action_deck_pc),
                        enabled = deckEnabled,
                        onClick = onOpenDeck,
                        modifier = Modifier.weight(1f).then(deckGating),
                    )
                    DeskActionButton(
                        label = stringResource(R.string.action_lock_pc),
                        enabled = device.online,
                        isSecondary = true,
                        onClick = { core.pcPower(device.id, sleep = false) },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun DeskActionButton(
    label: String,
    enabled: Boolean,
    isSecondary: Boolean = false,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    FilledTonalButton(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(WaypairDesign.controlRadius),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = if (isSecondary) MaterialTheme.colorScheme.surfaceContainerLow else MaterialTheme.colorScheme.surfaceContainerHigh,
            contentColor = if (isSecondary) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerLow,
            disabledContentColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
        ),
        border = BorderStroke(
            1.dp,
            if (isSecondary) MaterialTheme.colorScheme.outlineVariant else MaterialTheme.colorScheme.outline,
        ),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 13.dp),
        modifier = modifier.heightIn(min = WaypairDesign.touchTarget),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

@Composable
private fun DeskClockDisplay() {
    var nowMs by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            val current = System.currentTimeMillis()
            nowMs = current
            delay((1_000L - (current % 1_000L)).coerceIn(100L, 1_000L))
        }
    }

    val timeFormat = remember { SimpleDateFormat("HH:mm", Locale.getDefault()) }
    val secFormat = remember { SimpleDateFormat(":ss", Locale.getDefault()) }
    val dateFormat = remember { SimpleDateFormat("EEEE, MMMM d", Locale.getDefault()) }

    val minuteKey = nowMs / 60_000L
    val secondKey = nowMs / 1_000L
    val timeText = remember(minuteKey) { timeFormat.format(Date(minuteKey * 60_000L)) }
    val dateText = remember(minuteKey) { dateFormat.format(Date(minuteKey * 60_000L)) }
    val secText = remember(secondKey) { secFormat.format(Date(secondKey * 1_000L)) }

    Row(
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.Center,
    ) {
        Text(
            text = timeText,
            fontSize = 84.sp,
            lineHeight = 88.sp,
            fontWeight = FontWeight.Light,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Text(
            text = secText,
            fontSize = 28.sp,
            fontWeight = FontWeight.Normal,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(bottom = 12.dp, start = 4.dp),
        )
    }

    Text(
        text = dateText,
        style = MaterialTheme.typography.titleMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}



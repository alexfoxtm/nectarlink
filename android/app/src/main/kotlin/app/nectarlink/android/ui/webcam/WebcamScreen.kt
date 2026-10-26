// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.webcam

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.view.PreviewView
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import app.nectarlink.android.ui.theme.LinkButton as Button
import androidx.compose.material3.ButtonDefaults
import app.nectarlink.android.ui.theme.LinkFilledTonalButton as FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.detached
import app.nectarlink.android.ui.theme.WaypairDesign
import app.nectarlink.android.webcam.WebcamService
import java.util.Locale

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun WebcamScreen(
    device: Device,
    autoStart: Boolean = false,
    ownedEntryToken: String? = null,
    initialHeight: Int? = null,
    initialCamera: String? = null,
    onAutoStartConsumed: () -> Unit = {},
    onAccessChanged: () -> Unit = {},
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val view = LocalView.current
    val core = (context.applicationContext as app.nectarlink.android.NectarlinkApplication).core
    val requiresOwned = core.webcamRequiresOwned(device.id)
    var permissionEntryToken by remember { mutableStateOf<String?>(null) }
    val session by WebcamService.session.collectAsStateWithLifecycle()

    // Keep the phone's screen on while the webcam viewfinder/controls are open.
    DisposableEffect(view) {
        val prev = view.keepScreenOn
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = prev }
    }

    var hasCameraPermission by remember { mutableStateOf(WebcamService.hasPermission(context)) }
    var startAfterGrant by remember { mutableStateOf(false) }
    val askCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        hasCameraPermission = WebcamService.hasPermission(context)
        WebcamService.refreshCapabilities(context)
        onAccessChanged()
        if (granted && startAfterGrant) {
            startAfterGrant = false
            val original = permissionEntryToken
            permissionEntryToken = null
            if (original != null) WebcamService.startOwned(context, original)
            else if (!requiresOwned) WebcamService.start(
                context = context, pcId = device.id, height = initialHeight ?: session.height,
                camera = initialCamera ?: session.camera)
        } else {
            startAfterGrant = false
        }
    }

    LaunchedEffect(Unit) {
        WebcamService.refreshCapabilities(context)
    }

    LaunchedEffect(initialHeight, initialCamera) {
        // Owned admission uses the original Entry's Core options in the service.
        // A delayed notification must not configure whichever capture is current.
        if (!requiresOwned && ownedEntryToken == null) {
            if (initialHeight != null) WebcamService.setResolutionHeight(initialHeight)
            if (initialCamera != null) WebcamService.setCamera(initialCamera, context)
        }
    }

    LaunchedEffect(autoStart, hasCameraPermission, device.online) {
        if (autoStart) {
            onAutoStartConsumed()
            if (hasCameraPermission && device.online) {
                if (ownedEntryToken != null) WebcamService.startOwned(context, ownedEntryToken)
                else if (!requiresOwned) WebcamService.start(
                    context = context, pcId = device.id, height = initialHeight ?: session.height,
                    camera = initialCamera ?: session.camera)
            } else if (!hasCameraPermission) {
                startAfterGrant = true
                permissionEntryToken = ownedEntryToken
                askCamera.launch(Manifest.permission.CAMERA)
            }
        }
    }

    val previewView = remember {
        PreviewView(context).apply {
            scaleType = PreviewView.ScaleType.FILL_CENTER
            implementationMode = PreviewView.ImplementationMode.COMPATIBLE
        }
    }

    DisposableEffect(previewView) {
        WebcamService.attachUiPreview(previewView.surfaceProvider)
        onDispose {
            WebcamService.attachUiPreview(null)
        }
    }

    val isStreamingHere = session.active && session.pcId == device.id

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(WaypairDesign.spacingLg),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header: Back icon button, PC name, online dot
        item {
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
                Spacer(Modifier.width(4.dp))
                Text(
                    device.name,
                    style = MaterialTheme.typography.headlineSmall,
                    modifier = Modifier.weight(1f),
                )
                Surface(
                    shape = CircleShape,
                    color = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                    modifier = Modifier.size(10.dp),
                ) {}
            }
        }

        // Camera permission prompt card when CAMERA permission is not granted yet
        if (!hasCameraPermission) {
            item {
                Surface(
                    shape = RoundedCornerShape(WaypairDesign.cardRadius),
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Column(Modifier.fillMaxWidth().padding(WaypairDesign.spacingLg)) {
                        Text(
                            stringResource(R.string.webcam_permission_title),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            stringResource(R.string.webcam_permission_text, device.name),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                        Spacer(Modifier.height(12.dp))
                        Button(
                            modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                            onClick = {
                                startAfterGrant = device.online
                                askCamera.launch(Manifest.permission.CAMERA)
                            },
                        ) {
                            Text(stringResource(R.string.webcam_allow_camera))
                        }
                    }
                }
            }
        }

        // Offline notice
        if (!device.online) {
            item {
                Surface(
                    shape = RoundedCornerShape(WaypairDesign.cardRadius),
                    color = WaypairDesign.insetColor(),
                ) {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text(
                            stringResource(R.string.not_connected),
                            style = MaterialTheme.typography.titleSmall,
                        )
                        Spacer(Modifier.height(WaypairDesign.spacingXs))
                        Text(
                            stringResource(R.string.webcam_offline_hint, device.name),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        // Live 16:9 camera preview box
        item {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .aspectRatio(16f / 9f)
                    .clip(RoundedCornerShape(WaypairDesign.heroRadius))
                    .background(Color(0xFF171717)),
                contentAlignment = Alignment.Center,
            ) {
                if (isStreamingHere && hasCameraPermission) {
                    AndroidView(
                        factory = { previewView.detached() },
                        modifier = Modifier.fillMaxSize(),
                    )
                } else {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.padding(24.dp),
                    ) {
                        Text(
                            text = if (isStreamingHere && session.connecting) {
                                stringResource(R.string.webcam_status_connecting, device.name)
                            } else {
                                stringResource(R.string.webcam_status_ready, device.name)
                            },
                            style = MaterialTheme.typography.titleMedium,
                            color = Color.White,
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            text = stringResource(R.string.webcam_stream_spec, session.width, session.height, session.fps),
                            style = MaterialTheme.typography.bodySmall,
                            color = Color.White.copy(alpha = 0.72f),
                        )
                    }
                }

                // Overlay badge in top-left corner
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    color = Color.Black.copy(alpha = 0.58f),
                    contentColor = Color.White,
                    modifier = Modifier
                        .align(Alignment.TopStart)
                        .padding(12.dp),
                ) {
                    val camLabel = stringResource(
                        if (session.camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
                    )
                    Text(
                        text = if (isStreamingHere && !session.connecting) {
                            stringResource(
                                R.string.webcam_status_streaming_camera,
                                session.width,
                                session.height,
                                device.name,
                                camLabel,
                            )
                        } else {
                            stringResource(R.string.webcam_stream_camera_spec, session.width, session.height, camLabel)
                        },
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    )
                }
            }
        }

        // Controls Card: Start/Stop, Front/Back switch, Torch (when camera has flash), Resolution, Zoom
        item {
            Surface(
                shape = RoundedCornerShape(WaypairDesign.cardRadius),
                color = WaypairDesign.panelColor(),
                border = BorderStroke(1.dp, WaypairDesign.panelBorder()),
                contentColor = MaterialTheme.colorScheme.onSurface,
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(WaypairDesign.spacingLg),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    // Primary Start/Stop + Flip camera + Torch (only shown when camera has flash)
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        if (isStreamingHere) {
                            Button(
                                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                onClick = { WebcamService.stopForEntry(session.entryToken) },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.colorScheme.error,
                                    contentColor = MaterialTheme.colorScheme.onError,
                                ),
                            ) {
                                Text(stringResource(R.string.webcam_stop))
                            }
                        } else {
                            Button(
                                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                enabled = device.online && (!requiresOwned || ownedEntryToken != null),
                                onClick = {
                                    if (!hasCameraPermission) {
                                        startAfterGrant = true
                                        permissionEntryToken = ownedEntryToken
                                        askCamera.launch(Manifest.permission.CAMERA)
                                    } else {
                                        if (ownedEntryToken != null) WebcamService.startOwned(context, ownedEntryToken)
                                        else if (!requiresOwned) WebcamService.start(
                                            context = context, pcId = device.id,
                                            height = session.height, camera = session.camera)
                                    }
                                },
                            ) {
                                Text(stringResource(if (requiresOwned && ownedEntryToken == null) R.string.webcam_start_on_pc else R.string.webcam_start))
                            }
                        }

                        FilledTonalButton(
                            modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                            onClick = {
                                val next = if (session.camera == "front") "back" else "front"
                                WebcamService.setCamera(next, context)
                            },
                        ) {
                            val currentLabel = stringResource(
                                if (session.camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
                            )
                            Text(stringResource(R.string.webcam_switch_camera_current, currentLabel))
                        }

                        if (session.hasFlash) {
                            OutlinedButton(
                                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                enabled = isStreamingHere,
                                onClick = { WebcamService.setTorch(!session.torchOn) },
                            ) {
                                Text(
                                    stringResource(
                                        if (session.torchOn) R.string.webcam_torch_on else R.string.webcam_torch_off,
                                    ),
                                )
                            }
                        }
                    }

                    HorizontalDivider(color = WaypairDesign.panelBorder())

                    // Resolution picker: 720p, 1080p, and 4K (when supported)
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(
                            stringResource(R.string.webcam_resolution_label),
                            style = MaterialTheme.typography.labelLarge,
                        )
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingSm),
                            verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingSm),
                        ) {
                            val res720p = stringResource(R.string.webcam_res_720p)
                            val res1080p = stringResource(R.string.webcam_res_1080p)
                            val res4k = stringResource(R.string.webcam_res_4k)
                            val resolutions = buildList {
                                add(720 to res720p)
                                add(1080 to res1080p)
                                if (session.supports4k) add(2160 to res4k)
                            }
                            for ((h, label) in resolutions) {
                                FilterChip(
                                    modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                    selected = session.resolution == h,
                                    onClick = { WebcamService.setResolutionHeight(h) },
                                    label = { Text(label) },
                                )
                            }
                        }
                    }

                    // Zoom controls: presets + smooth high-contrast slider
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        val zoomLabel = stringResource(
                            R.string.webcam_zoom_label,
                            String.format(Locale.US, "%.1f", session.zoomRatio).toFloat(),
                        )
                        Text(
                            zoomLabel,
                            style = MaterialTheme.typography.labelLarge,
                        )
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(WaypairDesign.spacingSm),
                            verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingSm),
                        ) {
                            FilterChip(
                                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                selected = kotlin.math.abs(session.zoomRatio - 1f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(1f) },
                                label = { Text(stringResource(R.string.webcam_zoom_preset, 1)) },
                            )
                            FilterChip(
                                modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                selected = kotlin.math.abs(session.zoomRatio - 2f) < 0.08f,
                                onClick = { WebcamService.setZoomRatio(2f.coerceAtMost(session.maxZoomRatio)) },
                                label = { Text(stringResource(R.string.webcam_zoom_preset, 2)) },
                            )
                            if (session.maxZoomRatio >= 4f) {
                                FilterChip(
                                    modifier = Modifier.heightIn(min = WaypairDesign.touchTarget),
                                    selected = kotlin.math.abs(session.zoomRatio - 4f) < 0.08f,
                                    onClick = { WebcamService.setZoomRatio(4f.coerceAtMost(session.maxZoomRatio)) },
                                    label = { Text(stringResource(R.string.webcam_zoom_preset, 4)) },
                                )
                            }
                        }
                        val minZ = session.minZoomRatio
                        val maxZ = session.maxZoomRatio.coerceAtLeast(minZ + 0.1f)
                        Slider(
                            value = session.zoomRatio.coerceIn(minZ, maxZ),
                            onValueChange = { WebcamService.setZoomRatio(it) },
                            valueRange = minZ..maxZ,
                            colors = SliderDefaults.colors(
                                thumbColor = MaterialTheme.colorScheme.primary,
                                activeTrackColor = MaterialTheme.colorScheme.primary,
                                inactiveTrackColor = MaterialTheme.colorScheme.secondaryContainer,
                                disabledThumbColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                                disabledActiveTrackColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f),
                                disabledInactiveTrackColor = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.7f),
                            ),
                            modifier = Modifier.semantics { contentDescription = zoomLabel },
                        )
                    }
                }
            }
        }
    }
}

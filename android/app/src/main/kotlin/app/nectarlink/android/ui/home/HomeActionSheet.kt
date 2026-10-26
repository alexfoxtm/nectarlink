// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.net.Uri
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import app.nectarlink.android.ui.theme.LinkOutCubic
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.key
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import app.nectarlink.android.ui.theme.LocalLinkSheetInputFocus
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.ui.theme.LocalLinkSheetDismiss
import app.nectarlink.android.ui.theme.LinkButton
import app.nectarlink.android.ui.theme.LinkFilledTonalButton
import app.nectarlink.android.ui.theme.LinkModalSheet
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.android.ui.theme.WaypairDesign

/** Local discovery of existing actions; search and peer selection never invoke an action. */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
internal fun HomeActionSheet(
    state: CoreState,
    onDismiss: () -> Unit,
    onPairNew: () -> Unit,
    onRing: (String, Boolean) -> Unit,
    ringingPcIds: Set<String>,
    onSendFiles: (String, List<Uri>) -> Unit,
    onSendFolder: (String, Uri) -> Unit,
    onPower: (String, Boolean) -> Unit,
    onWake: (String) -> Unit,
    onRemote: (String) -> Unit,
    onDeck: (String) -> Unit,
    onDesk: (String) -> Unit,
    onRecord: (String) -> Unit,
    onWebcam: (String) -> Unit,
    onSetPcAudio: (String, Int?, Boolean?) -> Unit,
    onBrowsePcFiles: (String) -> Unit,
    micStreamingTo: String?,
    onStartMic: (String) -> Unit,
    onStopMic: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var selectedId by remember { mutableStateOf<String?>(null) }
    var pickerOpen by remember { mutableStateOf(false) }
    val computers = state.devices.filter { !it.isAndroid }
    val selectedDevice = computers.firstOrNull { it.id == selectedId }
    val reduced = LocalReducedMotion.current
    val direction = if (LocalLayoutDirection.current == LayoutDirection.Rtl) -1 else 1
    LaunchedEffect(selectedId, computers.map { it.id }) {
        if (selectedId != null && selectedDevice == null) {
            selectedId = null
            pickerOpen = false
        }
    }
    LinkModalSheet(
        onDismissRequest = onDismiss,
        onBackRequest = if (selectedId != null) ({ selectedId = null; query = "" }) else null,
    ) {
        val collapse = LocalLinkSheetDismiss.current
        val reportInputFocus = LocalLinkSheetInputFocus.current
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                if (selectedId != null) {
                    IconButton(onClick = { selectedId = null; query = "" }) {
                        Icon(painterResource(R.drawable.ic_back), stringResource(R.string.action_back), Modifier.size(24.dp))
                    }
                }
                Text(
                    selectedDevice?.name ?: stringResource(R.string.home_workspace_title),
                    style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f),
                )
                IconButton(onClick = collapse) {
                    Icon(painterResource(R.drawable.wp_close), stringResource(R.string.watch_close), Modifier.size(24.dp))
                }
            }
            OutlinedTextField(
                value = query, onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth().onFocusChanged { reportInputFocus(it.isFocused) },
                placeholder = { Text(stringResource(R.string.link_actions_search)) },
                leadingIcon = { Icon(painterResource(R.drawable.wp_link_search), null, Modifier.size(24.dp)) },
                shape = RoundedCornerShape(28.dp), singleLine = true,
            )
            if (selectedDevice != null) {
                LinkFilledTonalButton(onClick = { pickerOpen = true }) {
                    Icon(painterResource(R.drawable.wp_monitor), null, Modifier.size(24.dp))
                    Text(stringResource(R.string.settings_devices), Modifier.padding(start = 8.dp))
                }
            }
            AnimatedContent(
                targetState = selectedId,
                transitionSpec = {
                    if (reduced) fadeIn(tween(120)) togetherWith fadeOut(tween(120))
                    else {
                        val forward = targetState != null
                        (slideInHorizontally(tween(220, easing = LinkOutCubic)) { if (forward) it * direction else -it * direction } + fadeIn(tween(180))) togetherWith
                            (slideOutHorizontally(tween(220, easing = LinkOutCubic)) { if (forward) -it * direction else it * direction } + fadeOut(tween(180)))
                    }
                },
                label = "action_sheet_page",
            ) { pageId ->
                val pageDevice = computers.firstOrNull { it.id == pageId }
                Column(
                    Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    if (pageId == null && query.isBlank()) {
                        computers.forEach { device ->
                            Row(
                                Modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp))
                                    .background(WaypairDesign.panelColor())
                                    .clickable(role = Role.Button) { selectedId = device.id }
                                    .heightIn(min = 76.dp).padding(16.dp),
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Icon(painterResource(R.drawable.wp_monitor), null, Modifier.size(24.dp))
                                Column(Modifier.weight(1f)) {
                                    Text(device.name, style = MaterialTheme.typography.bodyLarge)
                                    Text(
                                        linkText(device.link, device.displayPath, device.usbMedium, device.routeTransport),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Icon(painterResource(R.drawable.wp_chevron), null, Modifier.size(20.dp).graphicsLayer { scaleX = direction.toFloat() })
                            }
                        }
                    } else {
                        val visibleDevices = if (pageId == null) computers else listOfNotNull(pageDevice)
                        visibleDevices.forEach { device ->
                            key(device.id) {
                            // Reuse the production card: one capability/permission matrix and one set of callbacks.
                            PcCard(
                                device = device, onRing = onRing, ringing = device.id in ringingPcIds,
                                onSendFiles = onSendFiles, onSendFolder = onSendFolder,
                                onPower = onPower, onWake = onWake,
                                onRemote = { onDismiss(); onRemote(it) },
                                onDeck = { onDismiss(); onDeck(it) },
                                onDesk = { onDismiss(); onDesk(it) },
                                onRecord = { onDismiss(); onRecord(it) },
                                onWebcam = { onDismiss(); onWebcam(it) },
                                onSetPcAudio = onSetPcAudio,
                                onBrowsePcFiles = { onDismiss(); onBrowsePcFiles(it) },
                                micOn = micStreamingTo == device.id,
                                onToggleMic = { if (it) onStartMic(device.id) else onStopMic() },
                                actionQuery = query,
                            )
                            }
                        }
                    }
                    LinkFilledTonalButton(onClick = { onDismiss(); onPairNew() }) {
                        Icon(painterResource(R.drawable.wp_qr), null, Modifier.size(24.dp))
                        Text(stringResource(R.string.action_pair_new), Modifier.padding(start = 8.dp))
                    }
                }
            }
        }
    }
    if (pickerOpen && selectedDevice != null) {
        // A pending local peer choice is invalidated whenever the owner or live peer list changes.
        var pendingId by remember(selectedId, computers.map { it.id }) { mutableStateOf(selectedId) }
        var appliedId by remember(selectedId, computers.map { it.id }) { mutableStateOf<String?>(null) }
        LinkModalSheet(
            onDismissRequest = {
                appliedId?.takeIf { id -> computers.any { it.id == id } }?.let { selectedId = it }
                pickerOpen = false
            },
            scrimColor = androidx.compose.ui.graphics.Color.Black.copy(alpha = 178f / 255f),
        ) {
            val collapse = LocalLinkSheetDismiss.current
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).selectableGroup().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(stringResource(R.string.settings_devices), style = MaterialTheme.typography.titleLarge)
                computers.forEach { device ->
                    Row(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp))
                            .background(WaypairDesign.panelColor())
                            .selectable(selected = pendingId == device.id, role = Role.RadioButton, onClick = { pendingId = device.id })
                            .heightIn(min = 64.dp).padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = pendingId == device.id, onClick = null)
                        Text(device.name, modifier = Modifier.padding(start = 12.dp))
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    LinkFilledTonalButton(onClick = collapse) { Text(stringResource(R.string.action_cancel)) }
                    LinkButton(
                        enabled = computers.any { it.id == pendingId },
                        onClick = {
                            appliedId = pendingId?.takeIf { id -> computers.any { it.id == id } }
                            collapse()
                        },
                    ) { Text(stringResource(R.string.link_actions_apply)) }
                }
            }
        }
    }
}

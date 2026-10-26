// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import app.nectarlink.android.ui.theme.WaypairDesign
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.Brush
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.nectarlink.android.ui.AppLanguage
import androidx.compose.material3.RadioButton
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import android.app.Activity
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.calls.PhoneCalls
import app.nectarlink.android.companion.PcCompanion
import app.nectarlink.android.contacts.PhoneContacts
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.PairingNotification
import app.nectarlink.android.mirror.InputService
import app.nectarlink.android.photos.RecentPhotos
import app.nectarlink.android.sms.PhoneSms
import app.nectarlink.android.storage.PhoneStorage
import app.nectarlink.android.toggles.PhoneToggles
import app.nectarlink.android.update.AppUpdater
import app.nectarlink.android.update.CheckForUpdates
import app.nectarlink.android.core.BackgroundAccess
import app.nectarlink.android.core.AppsPolicyStatus
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.Device
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import app.nectarlink.android.notifications.NotificationFilterPage
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.android.ui.guide.GuideHighlightTarget
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.LocalGlassTheme
import app.nectarlink.android.ui.theme.glassModalBlur
import app.nectarlink.android.ui.theme.glassModalContainerColor
import kotlinx.coroutines.delay

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SettingsScreen(
    state: CoreState,
    appearance: Appearance,
    onAppearance: ((Appearance) -> Appearance) -> Unit,
    touchpadSensitivity: Float,
    onTouchpadSensitivity: (Float) -> Unit,
    onUnpair: (String) -> Unit,
    onPairNew: () -> Unit,
    onAccessChanged: () -> Unit,
    updater: AppUpdater,
    onSetPhoneControlEnabled: (pcId: String, enabled: Boolean) -> Unit,
    onSetWayModeEnabled: (pcId: String, enabled: Boolean) -> Unit,
    onConfirmAppsAccess: (pcId: String) -> Unit,
    onRefreshAppsPolicy: (pcId: String) -> Unit,
    onResumePhoneControl: (pcId: String) -> Unit,
    settingsQualificationPcIds: Set<String> = emptySet(),
    onAllowSettingsWindowOnce: (pcId: String) -> Unit = {},
    onRevokeSettingsWindowAccess: (pcId: String) -> Unit = {},
    onSetStorageEnabled: (pcId: String, enabled: Boolean) -> Unit = { _, _ -> },
    onSetMicEnabled: (pcId: String, enabled: Boolean) -> Unit = { _, _ -> },
    onSetMirroringEnabled: (pcId: String, enabled: Boolean) -> Unit = { _, _ -> },
    onAddSafFolder: (Uri) -> Unit = {},
    onRemoveSafFolder: (String) -> Unit = {},
    onSetClipboardHistoryEnabled: (Boolean) -> Unit = {},
    onClearClipboardHistory: () -> Unit = {},
    suggestClipboardActions: Boolean = true,
    onSetSuggestClipboardActions: (Boolean) -> Unit = {},
    backgroundServiceEnabled: Boolean = true,
    onToggleBackgroundService: (Boolean) -> Unit = {},
    copiedLinkOffers: Boolean = false,
    onSetCopiedLinkOffers: (Boolean) -> Unit = {},
    onSetTimelineRetentionDays: (UInt) -> Unit = {},
    onClearTimeline: () -> Unit = {},
    onClearMessageCache: () -> Unit = {},
    onClearPhotoCache: () -> Unit = {},
    onClearReceivedFileHistory: () -> Unit = {},
    onClearEverything: () -> Unit = {},
    onToggleLocalSend: (Boolean) -> Unit = {},
    onToggleAwayMode: (Boolean) -> Unit = {},
    onOpenWalkthrough: () -> Unit = {},
    onOpenFeaturesGuide: () -> Unit = {},
    highlightTarget: GuideHighlightTarget? = null,
    onHighlightConsumed: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var notificationFilterOpen by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
    var confirmUnpair by remember { mutableStateOf<Device?>(null) }
    var confirmClearEverything by remember { mutableStateOf(false) }
    var pickLanguage by remember { mutableStateOf(false) }
    var showLicenses by remember { mutableStateOf(false) }
    var activeCategory by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf("all") }
    var focusedAppsPcId by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf<String?>(null) }
    var pulsingSectionKey by remember { mutableStateOf<String?>(null) }
    val isGlass = LocalGlassTheme.current
    val anyDialogOpen = confirmUnpair != null || confirmClearEverything || showLicenses

    val categories = remember {
        listOf(
            "all" to R.string.timeline_filter_all,
            "devices" to R.string.settings_devices,
            "apps_screen" to R.string.settings_apps_screen,
            "permissions" to R.string.walkthrough_permissions_title,
            "connection" to R.string.settings_connection,
            "appearance" to R.string.settings_appearance,
            "features" to R.string.settings_data_about,
        )
    }
    val appComputers = state.devices.filter { !it.isAndroid }

    androidx.compose.runtime.LaunchedEffect(activeCategory, state.status, appComputers.map { it.id }) {
        if ((activeCategory == "all" || activeCategory == "apps_screen") &&
            state.status is app.nectarlink.android.core.CoreStatus.Ready
        ) {
            appComputers.forEach { onRefreshAppsPolicy(it.id) }
        }
    }

    // Keep a screen of cards composed on each side, so scrolling back does not rebuild them.
    @OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
    val listState = androidx.compose.foundation.lazy.rememberLazyListState(
        cacheWindow = androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow(aheadFraction = 1f, behindFraction = 1f),
    )

    androidx.compose.runtime.LaunchedEffect(activeCategory, focusedAppsPcId) {
        val computerIndex = if (activeCategory == "apps_screen") {
            appComputers.indexOfFirst { it.id == focusedAppsPcId }
        } else -1
        listState.scrollToItem(if (computerIndex >= 0) computerIndex + 2 else 0)
    }

    androidx.compose.runtime.LaunchedEffect(highlightTarget) {
        val target = highlightTarget ?: return@LaunchedEffect
        val mapping = when (target) {
            GuideHighlightTarget.SETTINGS_DEVICES -> Triple("devices", "devices", 1)
            GuideHighlightTarget.SETTINGS_NOTIFICATIONS -> Triple("permissions", "notifications", 1)
            GuideHighlightTarget.SETTINGS_CONTROL -> Triple("apps_screen", "control", 1)
            GuideHighlightTarget.SETTINGS_BACKGROUND -> Triple("connection", "background", 2)
            GuideHighlightTarget.SETTINGS_CALLS_CONTACTS -> Triple("permissions", "calls_contacts", 3)
            GuideHighlightTarget.SETTINGS_PHOTOS_STORAGE -> Triple("permissions", "photos", 4)
            GuideHighlightTarget.SETTINGS_CLIPBOARD -> Triple("features", "clipboard", 1)
            GuideHighlightTarget.SETTINGS_LOCALSEND -> Triple("connection", "localsend", 3)
            GuideHighlightTarget.SETTINGS_DATA_STORAGE -> Triple("features", "data_storage", 2)
            else -> null
        }
        if (mapping != null) {
            val (category, sectionKey, indexInCat) = mapping
            focusedAppsPcId = null
            activeCategory = category
            pulsingSectionKey = sectionKey
            onHighlightConsumed()
            delay(60)
            runCatching { listState.animateScrollToItem(indexInCat) }
            delay(2200)
            if (pulsingSectionKey == sectionKey) {
                pulsingSectionKey = null
            }
        } else {
            onHighlightConsumed()
        }
    }

    if (notificationFilterOpen) {
        NotificationFilterPage(onBack = { notificationFilterOpen = false }, modifier = modifier)
    }
    if (!notificationFilterOpen) {
    LazyColumn(
        state = listState,
        modifier = modifier
            .fillMaxWidth()
            .wrapContentWidth(Alignment.CenterHorizontally)
            .widthIn(max = 600.dp)
            .glassModalBlur(isGlass && anyDialogOpen),
        contentPadding = PaddingValues(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 104.dp),
        verticalArrangement = Arrangement.spacedBy(WaypairDesign.spacingMd),
    ) {
        item(key = "header", contentType = "header") {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .shadow(
                            elevation = 8.dp,
                            shape = RoundedCornerShape(24.dp),
                            ambientColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.12f),
                            spotColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.18f),
                        )
                        .background(
                            brush = Brush.verticalGradient(
                                listOf(
                                    MaterialTheme.colorScheme.surfaceContainerHigh,
                                    MaterialTheme.colorScheme.surface,
                                ),
                            ),
                            shape = RoundedCornerShape(24.dp),
                        )
                        .padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        stringResource(R.string.settings_title),
                        style = MaterialTheme.typography.headlineMedium,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    TextButton(
                        onClick = onOpenFeaturesGuide,
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.heightIn(min = 48.dp),
                        contentPadding = PaddingValues(horizontal = 0.dp, vertical = 12.dp),
                    ) {
                        Text(stringResource(R.string.guide_title))
                    }
                }
                FlowRow(
                    modifier = Modifier.fillMaxWidth().selectableGroup(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    categories.forEach { (id, label) ->
                        val selected = activeCategory == id
                        val shape = RoundedCornerShape(16.dp)
                        Surface(
                            modifier = Modifier
                                .heightIn(min = 48.dp)
                                .shadow(
                                    elevation = if (selected) 1.dp else 4.dp,
                                    shape = shape,
                                    ambientColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.10f),
                                    spotColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.16f),
                                )
                                .clip(shape)
                                .selectable(
                                    selected = selected,
                                    role = Role.Tab,
                                    onClick = { focusedAppsPcId = null; activeCategory = id },
                                ),
                            shape = shape,
                            color = if (selected) MaterialTheme.colorScheme.surfaceContainerLow else MaterialTheme.colorScheme.surface,
                            contentColor = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        ) {
                            Text(
                                stringResource(label),
                                style = MaterialTheme.typography.labelLarge,
                                modifier = Modifier.padding(horizontal = 14.dp, vertical = 14.dp),
                            )
                        }
                    }
                }
            }
        }

        if (activeCategory == "all" || activeCategory == "devices") {
            item(key = "devices", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_devices),
                    highlighted = pulsingSectionKey == "devices",
                ) {
                    val context = LocalContext.current
                    if (state.devices.isEmpty()) {
                        Text(
                            stringResource(R.string.home_no_pc_paired),
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    state.devices.forEachIndexed { index, device ->
                        if (index > 0) {
                            SettingRowDivider()
                        }
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp), modifier = Modifier.fillMaxWidth()) {
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(device.name, style = MaterialTheme.typography.titleSmall)
                                    val dotColor = if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
                                    Row(
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    ) {
                                        Spacer(
                                            modifier = Modifier
                                                .size(8.dp)
                                                .drawBehind { drawCircle(dotColor) },
                                        )
                                        Text(
                                            stringResource(if (device.online) R.string.connected else R.string.not_connected),
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                }
                                OutlinedButton(onClick = { confirmUnpair = device }) { Text(stringResource(R.string.action_unpair)) }
                            }
                            val storageToggleTitle = stringResource(R.string.storage_pc_toggle)
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(storageToggleTitle, style = MaterialTheme.typography.bodyLarge)
                                    Text(
                                        stringResource(R.string.storage_pc_toggle_hint),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(
                                    checked = device.storageEnabled,
                                    onCheckedChange = { enabled ->
                                        onSetStorageEnabled(device.id, enabled)
                                        if (enabled && !state.storageAllFilesAccess && state.storageSafFolders.isEmpty()) {
                                            runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }
                                        }
                                    },
                                    colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                    modifier = Modifier.semantics { contentDescription = storageToggleTitle },
                                )
                            }
                            if (!device.isAndroid) {
                                val micToggleTitle = stringResource(R.string.mic_pc_toggle)
                                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                                    Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                        Text(micToggleTitle, style = MaterialTheme.typography.bodyLarge)
                                        Text(
                                            stringResource(R.string.mic_pc_toggle_hint),
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    Switch(
                                        checked = device.micEnabled,
                                        onCheckedChange = { enabled -> onSetMicEnabled(device.id, enabled) },
                                        colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                        modifier = Modifier.semantics { contentDescription = micToggleTitle },
                                    )
                                }
                                TextButton(onClick = {
                                    focusedAppsPcId = device.id
                                    activeCategory = "apps_screen"
                                }) {
                                    Text(stringResource(R.string.settings_apps_screen_open))
                                }
                            }
                        }
                    }
                    FilledTonalButton(onClick = onPairNew) { Text(stringResource(R.string.action_pair_new)) }
                }
            }
        }

        if (activeCategory == "all" || activeCategory == "apps_screen") {
            item(key = "control", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_apps_service),
                    highlighted = pulsingSectionKey == "control",
                ) {
                    Text(
                        stringResource(R.string.settings_apps_service_hint),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    ElevatedAccess()
                    SettingRowDivider()
                    ControlAccess(state.inputAccess)
                }
            }

            appComputers.forEach { device ->
                item(key = "apps_${device.id}", contentType = "section") {
                    Section(device.name, highlighted = focusedAppsPcId == device.id) {
                        Text(
                            stringResource(if (device.online) R.string.connected else R.string.not_connected),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (device.kind == app.nectarlink.core.DeviceKind.DESKTOP || device.kind == app.nectarlink.core.DeviceKind.LAPTOP) {
                            val appsPolicyStatus = if (state.status is app.nectarlink.android.core.CoreStatus.Ready) {
                                device.appsPolicyStatus
                            } else {
                                AppsPolicyStatus.UNKNOWN
                            }
                            Text(
                                stringResource(when (appsPolicyStatus) {
                                    AppsPolicyStatus.UNKNOWN -> R.string.settings_apps_status_unknown
                                    AppsPolicyStatus.OFF -> R.string.settings_apps_saved_off
                                    AppsPolicyStatus.ALLOWED -> R.string.settings_apps_saved_on
                                    AppsPolicyStatus.NEEDS_CONFIRMATION -> R.string.settings_apps_needs_confirmation
                                }),
                                style = MaterialTheme.typography.bodySmall,
                                color = if (appsPolicyStatus == AppsPolicyStatus.UNKNOWN ||
                                    appsPolicyStatus == AppsPolicyStatus.NEEDS_CONFIRMATION
                                ) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            if (appsPolicyStatus == AppsPolicyStatus.NEEDS_CONFIRMATION && device.wayModeEnabled) {
                                FilledTonalButton(onClick = { onConfirmAppsAccess(device.id) }) {
                                    Text(stringResource(R.string.settings_apps_confirm_access))
                                }
                            }
                        }
                        val mirroringToggleTitle = stringResource(R.string.mirroring_pc_toggle)
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                Text(mirroringToggleTitle, style = MaterialTheme.typography.bodyLarge)
                                Text(
                                    stringResource(R.string.mirroring_pc_toggle_hint),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Switch(
                                checked = device.mirroringEnabled,
                                onCheckedChange = { enabled -> onSetMirroringEnabled(device.id, enabled) },
                                colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                modifier = Modifier.semantics { contentDescription = mirroringToggleTitle },
                            )
                        }
                        if (device.kind == app.nectarlink.core.DeviceKind.DESKTOP || device.kind == app.nectarlink.core.DeviceKind.LAPTOP) {
                            val controlToggleTitle = stringResource(R.string.phone_control_pc_toggle)
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(controlToggleTitle, style = MaterialTheme.typography.bodyLarge)
                                    Text(
                                        stringResource(R.string.phone_control_pc_toggle_hint),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(
                                    checked = device.phoneControlEnabled,
                                    enabled = state.status is app.nectarlink.android.core.CoreStatus.Ready,
                                    onCheckedChange = { enabled -> onSetPhoneControlEnabled(device.id, enabled) },
                                    colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                    modifier = Modifier.semantics { contentDescription = controlToggleTitle },
                                )
                            }
                        }
                        if (device.kind == app.nectarlink.core.DeviceKind.DESKTOP || device.kind == app.nectarlink.core.DeviceKind.LAPTOP) {
                            val wayModeToggleTitle = stringResource(R.string.way_mode_toggle)
                            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                                    Text(wayModeToggleTitle, style = MaterialTheme.typography.bodyLarge)
                                    Text(
                                        stringResource(R.string.way_mode_toggle_hint),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                Switch(
                                    checked = device.wayModeEnabled,
                                    enabled = state.status is app.nectarlink.android.core.CoreStatus.Ready,
                                    onCheckedChange = { enabled -> onSetWayModeEnabled(device.id, enabled) },
                                    colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                                    modifier = Modifier.semantics { contentDescription = wayModeToggleTitle },
                                )
                            }
                        }
                        if (device.kind == app.nectarlink.core.DeviceKind.DESKTOP || device.kind == app.nectarlink.core.DeviceKind.LAPTOP) {
                            val settingsWindowActive = device.id in settingsQualificationPcIds
                            if (!device.wayModeEnabled || settingsWindowActive) {
                                val canArmSettingsWindow = state.status is app.nectarlink.android.core.CoreStatus.Ready &&
                                    device.online && device.mirroringEnabled && state.elevated &&
                                    Build.VERSION.SDK_INT >= Build.VERSION_CODES.R
                                Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
                                    Text(stringResource(R.string.settings_window_access_title), style = MaterialTheme.typography.bodyLarge)
                                    Text(
                                        stringResource(R.string.settings_window_access_hint),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                    if (settingsWindowActive) {
                                        OutlinedButton(onClick = { onRevokeSettingsWindowAccess(device.id) }) {
                                            Text(stringResource(R.string.settings_window_revoke))
                                        }
                                    } else {
                                        FilledTonalButton(
                                            enabled = canArmSettingsWindow,
                                            onClick = { onAllowSettingsWindowOnce(device.id) },
                                        ) {
                                            Text(stringResource(R.string.settings_window_allow_once))
                                        }
                                    }
                                }
                            }
                        }
                        if (device.id in state.typingStoppedPcIds) {
                            // Display gate only; the backend resume rechecks Node, epoch, Stop and scope.
                            val canResumeControl = state.status is app.nectarlink.android.core.CoreStatus.Ready &&
                                device.online && device.phoneControlEnabled &&
                                (device.mirroringEnabled || device.wayModeEnabled)
                            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text(
                                    stringResource(R.string.keyboard_recovery_required),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.error,
                                )
                                if (canResumeControl) {
                                    FilledTonalButton(
                                        onClick = { onResumePhoneControl(device.id) },
                                    ) {
                                        Text(stringResource(R.string.phone_control_resume))
                                    }
                                    Text(
                                        stringResource(R.string.phone_control_resume_hint),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                } else {
                                    Text(
                                        stringResource(R.string.phone_control_resume_unavailable),
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        }
                    }
                }
            }
            if (appComputers.isEmpty()) {
                item(key = "apps_empty", contentType = "section") {
                    Section(stringResource(R.string.settings_apps_screen)) {
                        Text(stringResource(R.string.settings_apps_screen_empty), style = MaterialTheme.typography.bodyLarge)
                        TextButton(onClick = { activeCategory = "devices" }) {
                            Text(stringResource(R.string.settings_devices))
                        }
                    }
                }
            }

            item(key = "remote", contentType = "section") {
                Section(stringResource(R.string.settings_remote)) {
                    val sensitivityTitle = stringResource(R.string.settings_touchpad_sensitivity)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Text(
                            sensitivityTitle,
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            stringResource(R.string.settings_touchpad_sensitivity_value, touchpadSensitivity),
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Slider(
                        value = touchpadSensitivity,
                        onValueChange = onTouchpadSensitivity,
                        valueRange = 0.5f..2.5f,
                        modifier = Modifier.semantics { contentDescription = sensitivityTitle },
                    )
                }
            }
        }

        if (activeCategory == "all" || activeCategory == "permissions") {
            item(key = "notifications", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_notifications),
                    highlighted = pulsingSectionKey == "notifications",
                ) {
                    NotificationAccess(state.notificationAccess, onChooseFilter = {
                        notificationFilterOpen = true
                    })
                }
            }

            item(key = "permissions_control", contentType = "section") {
                Section(stringResource(R.string.settings_control)) {
                    BankingPauseAccess()
                    SettingRowDivider()
                    DndAccess(state.dndAccess)
                    SettingRowDivider()
                    WriteSettingsAccess(state.writeSettingsAccess || state.elevated)
                }
            }

            item(key = "calls_contacts", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_calls_and_contacts),
                    highlighted = pulsingSectionKey == "calls_contacts",
                ) {
                    RuntimePermissionAccess(
                        title = stringResource(R.string.calls_title),
                        textOn = stringResource(R.string.calls_on),
                        textOff = stringResource(R.string.calls_off),
                        granted = state.callAccess,
                        permissions = PhoneCalls.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    RuntimePermissionAccess(
                        title = stringResource(R.string.contacts_title),
                        textOn = stringResource(R.string.contacts_on),
                        textOff = stringResource(R.string.contacts_off),
                        granted = state.contactsAccess,
                        permissions = PhoneContacts.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    RuntimePermissionAccess(
                        title = stringResource(R.string.sms_title),
                        textOn = stringResource(R.string.sms_on),
                        textOff = stringResource(R.string.sms_off),
                        granted = state.smsAccess,
                        permissions = PhoneSms.permissions,
                        onAccessChanged = onAccessChanged,
                    )
                }
            }

            item(key = "photos", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_photos),
                    highlighted = pulsingSectionKey == "photos",
                ) {
                    PhotosAccess(
                        granted = state.photoAccess,
                        partial = state.photoPartialAccess,
                        onAccessChanged = onAccessChanged,
                    )
                    SettingRowDivider()
                    StorageAccess(
                        allFiles = state.storageAllFilesAccess,
                        safFolders = state.storageSafFolders,
                        onAddSafFolder = onAddSafFolder,
                        onRemoveSafFolder = onRemoveSafFolder,
                    )
                }
            }
        }

        if (activeCategory == "all" || activeCategory == "connection") {
            item(key = "connection", contentType = "section") {
                Section(stringResource(R.string.settings_connection)) {
                    val mobileDataTitle = stringResource(R.string.guide_away_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(mobileDataTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_mobile_data_body),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = state.awayMode,
                            onCheckedChange = onToggleAwayMode,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = mobileDataTitle },
                        )
                    }
                }
            }

            item(key = "background", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_background_title),
                    highlighted = pulsingSectionKey == "background",
                ) {
                    val runInBackgroundTitle = stringResource(R.string.settings_run_in_background)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(runInBackgroundTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_run_in_background_hint),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = backgroundServiceEnabled,
                            onCheckedChange = onToggleBackgroundService,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = runInBackgroundTitle },
                        )
                    }
                    if (backgroundServiceEnabled && !state.backgroundUnrestricted) {
                        val context = LocalContext.current
                        val oemGuide = BackgroundAccess.oemGuideResId()?.let { stringResource(it) }
                        Column(
                            modifier = Modifier.fillMaxWidth().padding(start = 16.dp),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                stringResource(R.string.background_restriction_warning),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            if (oemGuide != null) {
                                Text(
                                    oemGuide,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            TextButton(onClick = {
                                runCatching { context.startActivity(BackgroundAccess.oemSettingsIntent(context)) }
                            }) {
                                Text(stringResource(R.string.battery_open_oem_settings))
                            }
                        }
                    }
                    SettingRowDivider()
                    CompanionWakeAccess(
                        associated = state.companionAssociated,
                        devices = state.devices,
                        onAccessChanged = onAccessChanged,
                    )
                }
            }

            item(key = "localsend", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_localsend_title),
                    highlighted = pulsingSectionKey == "localsend",
                ) {
                    val localsendTitle = stringResource(R.string.settings_localsend_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(localsendTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_localsend_subtitle),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = state.localsendEnabled,
                            onCheckedChange = onToggleLocalSend,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = localsendTitle },
                        )
                    }
                }
            }

            item(key = "local_hotspot", contentType = "section") {
                Section(stringResource(R.string.local_hotspot_title)) { LocalHotspotAccess() }
            }
        }

        if (activeCategory == "all" || activeCategory == "appearance") {
            item(key = "appearance", contentType = "section") {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        stringResource(R.string.settings_appearance),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 4.dp),
                    )
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .shadow(
                                elevation = 6.dp,
                                shape = RoundedCornerShape(24.dp),
                                ambientColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.10f),
                                spotColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.16f),
                            ),
                        shape = RoundedCornerShape(24.dp),
                        color = WaypairDesign.panelColor(),
                    ) {
                        Column(
                            modifier = Modifier.padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(24.dp),
                        ) {
                            AppearanceChoice(
                                label = stringResource(R.string.settings_mode),
                                options = listOf(
                                    "system" to stringResource(R.string.mode_system),
                                    "light" to stringResource(R.string.mode_light),
                                    "dark" to stringResource(R.string.mode_dark),
                                ),
                                selected = appearance.mode,
                                showModePreview = true,
                            ) { value -> onAppearance { it.copy(mode = value) } }
                            if (AppLanguage.supported) {
                                // System and English, plus the language picked from the list (or More).
                                val context = LocalContext.current
                                val language = remember { AppLanguage.current(context) }
                                val other = AppLanguage.languages.firstOrNull { it.first == language && it.first != "en" }
                                AppearanceChoice(
                                    label = stringResource(R.string.settings_language),
                                    options = listOf(
                                        "" to stringResource(R.string.mode_system),
                                        "en" to "English",
                                        (other ?: (MORE_LANGUAGES to stringResource(R.string.settings_language_more))),
                                    ),
                                    selected = language,
                                ) { value ->
                                    if (value == "" || value == "en") AppLanguage.set(context, value) else pickLanguage = true
                                }
                            }
                        }
                    }
                }
            }
        }

        if (activeCategory == "all" || activeCategory == "features") {
            item(key = "clipboard", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_clipboard_title),
                    highlighted = pulsingSectionKey == "clipboard",
                ) {
                    Text(
                        stringResource(R.string.settings_apps_service_clipboard_hint),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    TextButton(onClick = {
                        focusedAppsPcId = null
                        activeCategory = "apps_screen"
                    }) {
                        Text(stringResource(R.string.settings_apps_service_open))
                    }
                    SettingRowDivider()
                    val suggestionsTitle = stringResource(R.string.settings_clipboard_suggestions)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(suggestionsTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_clipboard_suggestions_hint),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = suggestClipboardActions,
                            onCheckedChange = onSetSuggestClipboardActions,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = suggestionsTitle },
                        )
                    }
                    SettingRowDivider()
                    val copiedLinksTitle = stringResource(R.string.settings_copied_link_offers)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(copiedLinksTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_copied_link_offers_hint),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Switch(
                            checked = copiedLinkOffers,
                            onCheckedChange = onSetCopiedLinkOffers,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = copiedLinksTitle },
                        )
                    }
                }
            }

            item(key = "data_storage", contentType = "section") {
                Section(
                    title = stringResource(R.string.settings_data_storage_title),
                    highlighted = pulsingSectionKey == "data_storage",
                ) {
                    // 1. Notification history
                    Column(Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.data_notifications_title), style = MaterialTheme.typography.titleSmall)
                        Text(
                            stringResource(R.string.data_notifications_subtitle),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    SettingRowDivider()

                    // 2. Clipboard history
                    val clipboardHistoryTitle = stringResource(R.string.data_clipboard_title)
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(clipboardHistoryTitle, style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_clipboard_subtitle),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            if (state.clipboardHistory.isNotEmpty()) {
                                Text(
                                    pluralStringResource(
                                        R.plurals.data_clipboard_count,
                                        state.clipboardHistory.size,
                                        state.clipboardHistory.size,
                                    ),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.primary,
                                    modifier = Modifier.padding(top = 2.dp),
                                )
                            }
                        }
                        Switch(
                            checked = state.clipboardHistoryEnabled,
                            onCheckedChange = onSetClipboardHistoryEnabled,
                            colors = app.nectarlink.android.ui.theme.nectarSwitchColors(),
                            modifier = Modifier.semantics { contentDescription = clipboardHistoryTitle },
                        )
                    }
                    if (state.clipboardHistory.isNotEmpty()) {
                        OutlinedButton(onClick = onClearClipboardHistory) {
                            Text(stringResource(R.string.data_action_clear))
                        }
                    }
                    SettingRowDivider()

                    // 3. Timeline
                    Text(stringResource(R.string.data_timeline_title), style = MaterialTheme.typography.titleSmall)
                    Text(
                        stringResource(R.string.data_timeline_subtitle),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Choice(
                        listOf(
                            "30" to stringResource(R.string.timeline_retention_30d),
                            "90" to stringResource(R.string.timeline_retention_90d),
                            "365" to stringResource(R.string.timeline_retention_1y),
                            "0" to stringResource(R.string.timeline_retention_all),
                        ),
                        state.timelineRetentionDays.toString(),
                    ) { value -> value.toUIntOrNull()?.let(onSetTimelineRetentionDays) }
                    if (state.timeline.isNotEmpty() || state.timelineTotal > 0u) {
                        OutlinedButton(onClick = onClearTimeline) {
                            Text(stringResource(R.string.data_action_clear))
                        }
                    }
                    SettingRowDivider()

                    // 4. Message cache
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_messages_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_messages_subtitle),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.cachedMmsFiles > 0) {
                            OutlinedButton(onClick = onClearMessageCache) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 5. Photo thumbnails & camera cache
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_photos_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_photos_subtitle),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.cachedPhotoFiles > 0) {
                            OutlinedButton(onClick = onClearPhotoCache) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 6. Received-file history
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.data_received_files_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.data_received_files_subtitle),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (state.receivedFileRecords > 0 || state.transfers.isNotEmpty()) {
                            OutlinedButton(onClick = onClearReceivedFileHistory) {
                                Text(stringResource(R.string.data_action_clear))
                            }
                        }
                    }
                    SettingRowDivider()

                    // 7. Clear everything
                    Text(
                        stringResource(R.string.data_clear_everything_hint),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    OutlinedButton(onClick = { confirmClearEverything = true }) {
                        Text(stringResource(R.string.data_clear_everything_button))
                    }
                }
            }

            item(key = "waypair_extras", contentType = "section") {
                Section(stringResource(R.string.waypair_extras_title)) {
                    Text(
                        stringResource(R.string.waypair_extras_body),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    SettingRowDivider()
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.waypair_extras_filters_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.waypair_extras_filters_body),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        OutlinedButton(onClick = { notificationFilterOpen = true }) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Column(Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.waypair_extras_ring_title), style = MaterialTheme.typography.titleSmall)
                        Text(
                            stringResource(R.string.waypair_extras_ring_body),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    SettingRowDivider()
                    Column(Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.waypair_extras_updates_title), style = MaterialTheme.typography.titleSmall)
                        Text(
                            stringResource(R.string.waypair_extras_updates_body),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            item(key = "about", contentType = "section") {
                Section(stringResource(R.string.settings_about)) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.guide_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_guide_card_body),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        FilledTonalButton(onClick = onOpenFeaturesGuide) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.guide_open_walkthrough), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.settings_walkthrough_body),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        OutlinedButton(onClick = onOpenWalkthrough) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                        Column(Modifier.weight(1f).padding(end = 12.dp)) {
                            Text(stringResource(R.string.about_licenses_title), style = MaterialTheme.typography.titleSmall)
                            Text(
                                stringResource(R.string.about_licenses_body),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        OutlinedButton(onClick = { showLicenses = true }) {
                            Text(stringResource(R.string.clip_action_open))
                        }
                    }
                    SettingRowDivider()
                    Text(stringResource(R.string.about_version, "${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})"), style = MaterialTheme.typography.bodyLarge)
                    CheckForUpdates(updater)
                }
            }
        }
    }

    if (showLicenses) {
        LicensesDialog(onDismiss = { showLicenses = false })
    }

    confirmUnpair?.let { device ->
        AlertDialog(
            onDismissRequest = { confirmUnpair = null },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.unpair_title, device.name)) },
            text = { Text(stringResource(R.string.unpair_text)) },
            confirmButton = {
                TextButton(onClick = { onUnpair(device.id); confirmUnpair = null }) {
                    Text(stringResource(R.string.action_unpair))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmUnpair = null }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (pickLanguage && AppLanguage.supported) {
        val context = LocalContext.current
        val language = remember { AppLanguage.current(context) }
        AlertDialog(
            onDismissRequest = { pickLanguage = false },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.settings_language)) },
            text = {
                Column {
                    AppLanguage.languages.forEach { (tag, name) ->
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .selectable(selected = tag == language, role = Role.RadioButton) {
                                    pickLanguage = false
                                    AppLanguage.set(context, tag)
                                }
                                .padding(vertical = 12.dp),
                        ) {
                            RadioButton(selected = tag == language, onClick = null)
                            Spacer(Modifier.width(12.dp))
                            Text(name, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { pickLanguage = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }

    if (confirmClearEverything) {
        AlertDialog(
            onDismissRequest = { confirmClearEverything = false },
            containerColor = glassModalContainerColor(),
            title = { Text(stringResource(R.string.data_clear_everything_dialog_title)) },
            text = { Text(stringResource(R.string.data_clear_everything_dialog_body)) },
            confirmButton = {
                TextButton(onClick = {
                    onClearEverything()
                    confirmClearEverything = false
                }) {
                    Text(stringResource(R.string.data_clear_everything_confirm))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmClearEverything = false }) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }
    }
}

private tailrec fun Context.findActivity(): Activity? = when (this) {
    is Activity -> this
    is android.content.ContextWrapper -> baseContext.findActivity()
    else -> null
}

private fun openAppInfoSettings(context: Context) {
    runCatching {
        context.startActivity(
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

private fun openDeveloperOptionsSettings(context: Context) {
    val devIntent = Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    if (runCatching { context.startActivity(devIntent) }.isFailure) {
        runCatching {
            context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}

/**
 * Notification access, which mirroring needs. Android shows the switch in
 * its own settings; sideloaded apps on Android 13+ must first be allowed
 * "restricted settings" there.
 */
@Composable
private fun NotificationAccess(granted: Boolean, onChooseFilter: () -> Unit) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.notifications_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.notifications_on else R.string.notifications_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(NotificationListener.settingsIntent(context)) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    TextButton(onClick = onChooseFilter, modifier = Modifier.padding(top = 4.dp)) {
        Text(stringResource(R.string.waypair_extras_filters_action))
    }
    if (!granted && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = { openAppInfoSettings(context) }) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

/** Companion Device Manager association for waking when a PC BLE beacon is nearby. */
@Composable
private fun CompanionWakeAccess(
    associated: Boolean,
    devices: List<Device>,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val chooserLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartIntentSenderForResult(),
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            PcCompanion.startObservingPresence(context)
        }
        onAccessChanged()
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.companion_wake_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (associated) R.string.companion_wake_on else R.string.companion_wake_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val primaryPc = devices.firstOrNull { it.online } ?: devices.firstOrNull()
        val triggerAssociate = {
            val activity = context.findActivity()
            if (activity != null) {
                PcCompanion.requestAssociation(
                    activity = activity,
                    pcName = primaryPc?.name ?: "PC",
                    pcDeviceIdHex = primaryPc?.id,
                    onResult = { onAccessChanged() },
                    onLauncherFallback = { sender ->
                        runCatching {
                            chooserLauncher.launch(IntentSenderRequest.Builder(sender).build())
                        }
                    },
                )
            }
        }
        if (associated) {
            OutlinedButton(onClick = triggerAssociate) {
                Text(stringResource(R.string.action_manage))
            }
        } else {
            Button(onClick = triggerAssociate) {
                Text(stringResource(R.string.companion_wake_associate))
            }
        }
    }
}

/** Do Not Disturb and silent ringer control from a PC. */
@Composable
private fun DndAccess(granted: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.dnd_access_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.dnd_access_on else R.string.dnd_access_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneToggles.dndSettingsIntent()) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Screen brightness control from a PC (Modify system settings, or Elevated). */
@Composable
private fun WriteSettingsAccess(granted: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.write_settings_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (granted) R.string.write_settings_on else R.string.write_settings_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneToggles.writeSettingsIntent(context)) }.let {} }
        if (granted) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

/** Control from the PC: NectarLink's accessibility service, turned on in Android's settings. */
@Composable
private fun ControlAccess(on: Boolean) {
    val context = LocalContext.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.control_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (on) R.string.control_on else R.string.control_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(InputService.settingsIntent()) }.let {} }
        if (on) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (!on && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f).padding(end = 12.dp),
            )
            OutlinedButton(onClick = { openAppInfoSettings(context) }) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

/** Elevated through the phone's own wireless debugging: real touch and keys. */
@Composable
private fun ElevatedAccess() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by Elevated.state.collectAsStateWithLifecycle()
    var steps by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.settings_apps_service_status), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(
                        when (state) {
                            Elevated.State.NotSetUp -> R.string.settings_apps_service_not_set_up
                            Elevated.State.Waiting -> R.string.settings_apps_service_waiting
                            Elevated.State.Starting -> R.string.settings_apps_service_starting
                            Elevated.State.Running ->
                                if (Elevated.rootActive) R.string.settings_apps_service_running_root else R.string.settings_apps_service_running
                            is Elevated.State.Failed -> when ((state as Elevated.State.Failed).reason) {
                                Elevated.Reason.NotPaired -> R.string.settings_apps_service_not_paired
                                Elevated.Reason.RootDenied -> R.string.settings_apps_service_root_failed
                                else -> R.string.settings_apps_service_failed
                            }
                        },
                    ),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            when (state) {
                Elevated.State.NotSetUp, is Elevated.State.Failed -> Button(onClick = { steps = true }) {
                    Text(stringResource(R.string.elevated_set_up))
                }
                Elevated.State.Waiting -> Button(onClick = {
                    scope.launch {
                        PairingNotification.show(context)
                        if (!Elevated.start()) {
                            openDeveloperOptionsSettings(context)
                        }
                    }
                }) {
                    Text(stringResource(R.string.elevated_try_again))
                }
                Elevated.State.Running -> OutlinedButton(onClick = { scope.launch { Elevated.forget() } }) {
                    Text(stringResource(R.string.elevated_turn_off))
                }
                Elevated.State.Starting -> {}
            }
        }
        if (state == Elevated.State.Waiting || state is Elevated.State.Failed) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = {
                    PairingNotification.show(context)
                    openDeveloperOptionsSettings(context)
                }) {
                    Text(stringResource(R.string.elevated_open_developer))
                }
            }
        }
        // Root only when the user picks it, and only on a rooted phone.
        val rootOffered = Elevated.isRootAvailable() &&
            (state == Elevated.State.NotSetUp || state is Elevated.State.Failed)
        if (rootOffered) {
            Text(
                stringResource(R.string.elevated_root_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            OutlinedButton(onClick = { scope.launch { Elevated.setUpWithRoot() } }) {
                Text(stringResource(R.string.elevated_use_root))
            }
        }
    }
    if (steps) {
        AlertDialog(
            onDismissRequest = { steps = false },
            title = { Text(stringResource(R.string.elevated_steps_title)) },
            text = { Text(stringResource(R.string.elevated_steps)) },
            confirmButton = {
                TextButton(onClick = {
                    steps = false
                    PairingNotification.show(context)
                    openDeveloperOptionsSettings(context)
                }) { Text(stringResource(R.string.elevated_open_developer)) }
            },
            dismissButton = {
                TextButton(onClick = { steps = false }) { Text(stringResource(R.string.action_cancel)) }
            },
        )
    }
}

@Composable
private fun RuntimePermissionAccess(
    title: String,
    textOn: String,
    textOff: String,
    granted: Boolean,
    permissions: Array<String>,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        onAccessChanged()
        if (grants.values.none { it }) {
            openAppInfoSettings(context)
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(
                if (granted) textOn else textOff,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (granted) {
            OutlinedButton(onClick = { openAppInfoSettings(context) }) {
                Text(stringResource(R.string.action_manage))
            }
        } else {
            Button(onClick = { ask.launch(permissions) }) { Text(stringResource(R.string.action_allow)) }
        }
    }
}

@Composable
private fun PhotosAccess(
    granted: Boolean,
    partial: Boolean,
    onAccessChanged: () -> Unit,
) {
    val context = LocalContext.current
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        onAccessChanged()
        if (grants.values.none { it }) {
            openAppInfoSettings(context)
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.photos_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(
                    when {
                        partial -> R.string.photos_partial
                        granted -> R.string.photos_on
                        else -> R.string.photos_off
                    },
                ),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when {
            partial -> OutlinedButton(onClick = { ask.launch(RecentPhotos.permissions) }) {
                Text(stringResource(R.string.action_select_photos))
            }
            granted -> OutlinedButton(onClick = { openAppInfoSettings(context) }) {
                Text(stringResource(R.string.action_manage))
            }
            else -> Button(onClick = { ask.launch(RecentPhotos.permissions) }) {
                Text(stringResource(R.string.action_allow))
            }
        }
    }
}

@Composable
private fun StorageAccess(
    allFiles: Boolean,
    safFolders: List<String>,
    onAddSafFolder: (Uri) -> Unit,
    onRemoveSafFolder: (String) -> Unit,
) {
    val context = LocalContext.current
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) onAddSafFolder(uri)
    }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(stringResource(R.string.storage_all_files_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (allFiles) R.string.storage_all_files_on else R.string.storage_all_files_off),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        val open = { runCatching { context.startActivity(PhoneStorage.allFilesAccessIntent(context)) }.let {} }
        if (allFiles) {
            OutlinedButton(onClick = open) { Text(stringResource(R.string.action_manage)) }
        } else {
            Button(onClick = open) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (!allFiles) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
            Column(Modifier.weight(1f).padding(end = 12.dp)) {
                Text(stringResource(R.string.storage_saf_title), style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(R.string.storage_saf_hint),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            OutlinedButton(onClick = { pickFolder.launch(null) }) {
                Text(stringResource(R.string.storage_add_folder))
            }
        }
        safFolders.forEach { name ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp)) {
                Text(name, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                TextButton(onClick = { onRemoveSafFolder(name) }) {
                    Text(stringResource(R.string.storage_remove_folder))
                }
            }
        }
    }
}

@Composable
private fun SettingRowDivider() {
    HorizontalDivider(
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.25f),
    )
}

@Composable
private fun Section(
    title: String,
    highlighted: Boolean = false,
    content: @Composable () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            color = if (highlighted) scheme.primary else scheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp),
        )
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .shadow(
                    elevation = 6.dp,
                    shape = RoundedCornerShape(24.dp),
                    ambientColor = scheme.primary.copy(alpha = 0.10f),
                    spotColor = scheme.primary.copy(alpha = 0.16f),
                ),
            shape = RoundedCornerShape(24.dp),
            color = if (highlighted) scheme.secondaryContainer else WaypairDesign.panelColor(),
            border = if (highlighted) BorderStroke(1.dp, scheme.primary.copy(alpha = 0.5f)) else null,
        ) {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                content()
            }
        }
    }
}

/** Content-sized options keep labels readable as pane width and text size change. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun AppearanceChoice(
    label: String,
    options: List<Pair<String, String>>,
    selected: String,
    maxItemsInEachRow: Int = Int.MAX_VALUE,
    showModePreview: Boolean = false,
    onSelect: (String) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (label.isNotEmpty()) {
            Text(label, style = MaterialTheme.typography.titleSmall, color = scheme.onSurfaceVariant)
        }
        FlowRow(
            modifier = Modifier.fillMaxWidth().selectableGroup(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            maxItemsInEachRow = maxItemsInEachRow,
        ) {
            options.forEach { (value, text) ->
                val isSelected = value == selected
                val shape = RoundedCornerShape(16.dp)
                Surface(
                    modifier = Modifier
                        .widthIn(min = 104.dp)
                        .heightIn(min = 48.dp)
                        .shadow(
                            elevation = if (isSelected) 1.dp else 4.dp,
                            shape = shape,
                            ambientColor = scheme.primary.copy(alpha = 0.10f),
                            spotColor = scheme.primary.copy(alpha = 0.16f),
                        )
                        .clip(shape)
                        .selectable(
                            selected = isSelected,
                            role = Role.RadioButton,
                            onClick = { onSelect(value) },
                        ),
                    shape = shape,
                    color = if (isSelected) scheme.surfaceContainerLow else scheme.surfaceContainerHigh,
                    contentColor = if (isSelected) scheme.primary else scheme.onSurface,
                    border = if (isSelected) BorderStroke(1.dp, scheme.primary.copy(alpha = 0.35f)) else null,
                ) {
                    Column(
                        modifier = Modifier.padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        if (showModePreview) {
                            androidx.compose.foundation.layout.Box(
                                modifier = Modifier
                                    .size(width = 88.dp, height = 44.dp)
                                    .clip(RoundedCornerShape(6.dp))
                                    .drawBehind {
                                        val light = androidx.compose.ui.graphics.Color(0xFFD8DCF0)
                                        val dark = androidx.compose.ui.graphics.Color(0xFF25284D)
                                        drawRect(if (value == "dark") dark else light)
                                        if (value == "system") {
                                            drawRect(
                                                color = dark,
                                                topLeft = androidx.compose.ui.geometry.Offset(size.width / 2f, 0f),
                                            )
                                        }
                                        drawRect(
                                            color = androidx.compose.ui.graphics.Color(0xFF3E48AA),
                                            topLeft = androidx.compose.ui.geometry.Offset(8.dp.toPx(), 8.dp.toPx()),
                                            size = androidx.compose.ui.geometry.Size(24.dp.toPx(), 4.dp.toPx()),
                                        )
                                        drawRect(
                                            color = androidx.compose.ui.graphics.Color(0xFFA9AEFF).copy(alpha = 0.35f),
                                            topLeft = androidx.compose.ui.geometry.Offset(8.dp.toPx(), 20.dp.toPx()),
                                            size = androidx.compose.ui.geometry.Size(size.width - 16.dp.toPx(), 16.dp.toPx()),
                                        )
                                    }
                                    .clearAndSetSemantics {},
                            )
                        }
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(text, style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f, fill = false))
                            androidx.compose.foundation.layout.Box(
                                modifier = Modifier.size(20.dp),
                                contentAlignment = Alignment.Center,
                            ) {
                                if (isSelected) {
                                    androidx.compose.foundation.Canvas(Modifier.size(20.dp).clearAndSetSemantics {}) {
                                        val ink = scheme.primary
                                        val stroke = 2.dp.toPx()
                                        drawLine(ink, androidx.compose.ui.geometry.Offset(size.width * 0.2f, size.height * 0.5f), androidx.compose.ui.geometry.Offset(size.width * 0.42f, size.height * 0.72f), stroke, androidx.compose.ui.graphics.StrokeCap.Round)
                                        drawLine(ink, androidx.compose.ui.geometry.Offset(size.width * 0.42f, size.height * 0.72f), androidx.compose.ui.geometry.Offset(size.width * 0.8f, size.height * 0.28f), stroke, androidx.compose.ui.graphics.StrokeCap.Round)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** The third language choice when it isn't a picked language: opens the list. */
private const val MORE_LANGUAGES = "more"

@Composable
private fun Choice(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    AppearanceChoice(
        label = "",
        options = options,
        selected = selected,
        maxItemsInEachRow = 2,
        onSelect = onSelect,
    )
}


@Composable
private fun LocalHotspotAccess() {
    val context = LocalContext.current
    val controller = remember { app.nectarlink.android.toggles.LocalHotspotController(context) }
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    val status by controller.status.collectAsStateWithLifecycle()
    val elevated by Elevated.state.collectAsStateWithLifecycle()
    androidx.compose.runtime.DisposableEffect(lifecycle, controller, elevated) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            when (event) {
                androidx.lifecycle.Lifecycle.Event.ON_RESUME -> controller.start()
                androidx.lifecycle.Lifecycle.Event.ON_PAUSE -> controller.stop()
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        if (lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED)) controller.start()
        onDispose { lifecycle.removeObserver(observer); controller.stop() }
    }
    val reason = stringResource(when (status) {
        app.nectarlink.android.toggles.LocalHotspotController.Status.CHECKING -> R.string.local_hotspot_checking
        app.nectarlink.android.toggles.LocalHotspotController.Status.READY -> R.string.local_hotspot_ready
        app.nectarlink.android.toggles.LocalHotspotController.Status.BUSY -> R.string.local_hotspot_busy
        app.nectarlink.android.toggles.LocalHotspotController.Status.ACTIVE -> R.string.local_hotspot_active
        app.nectarlink.android.toggles.LocalHotspotController.Status.UNAVAILABLE -> R.string.local_hotspot_unavailable
        app.nectarlink.android.toggles.LocalHotspotController.Status.DENIED -> R.string.local_hotspot_denied
        app.nectarlink.android.toggles.LocalHotspotController.Status.PAUSED -> R.string.local_hotspot_paused
        app.nectarlink.android.toggles.LocalHotspotController.Status.CONFIG_CHANGED -> R.string.local_hotspot_changed
        app.nectarlink.android.toggles.LocalHotspotController.Status.CANCELLED -> R.string.local_hotspot_cancelled
        app.nectarlink.android.toggles.LocalHotspotController.Status.FAILED -> R.string.local_hotspot_failed
    })
    Text(reason, style = MaterialTheme.typography.bodyLarge)
    Text(stringResource(R.string.local_hotspot_ios), style = MaterialTheme.typography.bodyMedium)
    val enabled = status in setOf(
        app.nectarlink.android.toggles.LocalHotspotController.Status.READY,
        app.nectarlink.android.toggles.LocalHotspotController.Status.CONFIG_CHANGED,
        app.nectarlink.android.toggles.LocalHotspotController.Status.FAILED,
        app.nectarlink.android.toggles.LocalHotspotController.Status.CANCELLED,
    )
    Button(onClick = { controller.enable() }, enabled = enabled,
        modifier = Modifier.semantics { contentDescription = reason }) {
        Text(stringResource(if (status == app.nectarlink.android.toggles.LocalHotspotController.Status.BUSY) R.string.local_hotspot_busy else R.string.local_hotspot_enable))
    }
    if (status in setOf(app.nectarlink.android.toggles.LocalHotspotController.Status.UNAVAILABLE,
            app.nectarlink.android.toggles.LocalHotspotController.Status.DENIED,
            app.nectarlink.android.toggles.LocalHotspotController.Status.PAUSED)) {
        OutlinedButton(onClick = { controller.stop(); controller.start() }) { Text(stringResource(R.string.local_hotspot_refresh)) }
    }
    if (status == app.nectarlink.android.toggles.LocalHotspotController.Status.ACTIVE) {
        OutlinedButton(onClick = { runCatching { context.startActivity(Intent(Settings.ACTION_WIRELESS_SETTINGS)) } }) {
            Text(stringResource(R.string.local_hotspot_credentials))
        }
    }
}

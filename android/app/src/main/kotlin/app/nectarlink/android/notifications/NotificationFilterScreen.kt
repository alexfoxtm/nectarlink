// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.ui.theme.nectarSwitchColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Per-app and per-type mirror choices. The settings host shows this page;
 * this file does not grant SMS or Gmail access.
 */
@Composable
fun NotificationFilterPage(onBack: () -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val store = remember { NotificationFilterStore.create(context) }
    var filter by remember { mutableStateOf(store.load()) }
    var notice by remember { mutableStateOf<String?>(null) }
    var choosingApps by rememberSaveable { mutableStateOf(false) }
    val back = {
        if (choosingApps) choosingApps = false else onBack()
    }
    BackHandler(onBack = back)
    if (choosingApps) {
        NotificationAppChooser(
            store = store,
            filter = filter,
            notice = notice,
            modifier = modifier,
            onFilter = { next ->
                if (store.save(next)) {
                    filter = next
                    notice = null
                    NotificationListener.instance?.applyFilter()
                } else {
                    notice = "Could not save these choices. The previous selection is still in effect."
                }
            },
            onBack = { choosingApps = false },
        )
        return
    }
    LazyColumn(
        modifier = modifier.fillMaxSize().padding(horizontal = 20.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            TextButton(onClick = onBack) { Text(stringResource(R.string.action_back)) }
            Text("Notification mirroring", style = MaterialTheme.typography.headlineSmall)
            Text(
                "Choose which apps and notification types may be mirrored. New apps are included until you switch to selected apps.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        item {
            FilterSwitch(
                title = "Mirror notifications",
                summary = "On by default, same as today. Off stops new mirrored notifications. Your app and type choices stay saved.",
                checked = filter.enabled,
                onCheckedChange = { save(store, filter.copy(enabled = it), { filter = it }, { notice = it }) },
            )
        }
        notice?.let { text ->
            item {
                Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
        }
        item {
            Text("Apps", style = MaterialTheme.typography.titleSmall)
            ModeRow(
                title = "All apps",
                summary = "Include apps you install later",
                selected = filter.appMode == NotificationAppMode.ALL,
                onClick = { save(store, filter.copy(appMode = NotificationAppMode.ALL), { filter = it }, { notice = it }) },
            )
            ModeRow(
                title = "Selected apps",
                summary = "Only the apps you turn on",
                selected = filter.appMode == NotificationAppMode.SELECTED,
                onClick = { save(store, filter.copy(appMode = NotificationAppMode.SELECTED), { filter = it }, { notice = it }) },
            )
            if (filter.appMode == NotificationAppMode.SELECTED) {
                TextButton(onClick = { choosingApps = true }) {
                    Text(selectedCount(filter.selectedPackages.size))
                }
                Text(
                    "New apps stay off until you select them.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item {
            Text("Types", style = MaterialTheme.typography.titleSmall)
            Text(
                "Types use Android's notification category. Other is everything that is not a message or email category.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        items(NotificationKind.entries, key = { it.name }) { kind ->
            FilterSwitch(
                title = kindTitle(kind),
                summary = kindSummary(kind),
                checked = kind in filter.kinds,
                onCheckedChange = { on ->
                    val kinds = if (on) filter.kinds + kind else filter.kinds - kind
                    save(store, filter.copy(kinds = kinds), { filter = it }, { notice = it })
                },
            )
        }
        item {
            Text(
                "This list does not grant SMS or Gmail access and does not read or log message text. An SMS inbox or Gmail code reader, if you turn one on separately, is not changed by these switches.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun NotificationAppChooser(
    store: NotificationFilterStore,
    filter: NotificationFilter,
    notice: String?,
    onFilter: (NotificationFilter) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var query by rememberSaveable { mutableStateOf("") }
    var apps by remember { mutableStateOf<List<NotificationAppChoice>>(emptyList()) }
    LaunchedEffect(filter.selectedPackages) {
        val selected = filter.selectedPackages
        val seen = store.observed()
        apps = withContext(Dispatchers.IO) { notificationAppChoices(context, selected, seen) }
    }
    val shown = remember(apps, query) {
        val search = query.trim()
        if (search.isEmpty()) apps else apps.filter {
            it.label.contains(search, ignoreCase = true) || it.packageName.contains(search, ignoreCase = true)
        }
    }
    LazyColumn(
        modifier = modifier.fillMaxSize().padding(horizontal = 20.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            TextButton(onClick = onBack) { Text(stringResource(R.string.action_back)) }
            Text("Selected apps", style = MaterialTheme.typography.headlineSmall)
            Text(
                selectedCount(filter.selectedPackages.size) + ". Changes save as you choose.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            BasicTextField(
                value = query,
                onValueChange = { query = it },
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
                decorationBox = { inner ->
                    Box(Modifier.fillMaxWidth()) {
                        if (query.isEmpty()) {
                            Text("Search apps", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        inner()
                    }
                },
            )
            notice?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
        }
        if (shown.isEmpty()) {
            item {
                Text(
                    if (query.isBlank()) "No apps yet. They also appear here after they post a notification."
                    else "No apps match your search.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        items(shown, key = { it.packageName }) { app ->
            FilterSwitch(
                title = app.label,
                summary = app.packageName,
                checked = app.packageName in filter.selectedPackages,
                onCheckedChange = { on ->
                    val packages = if (on) filter.selectedPackages + app.packageName else filter.selectedPackages - app.packageName
                    onFilter(filter.copy(selectedPackages = packages))
                },
            )
        }
    }
}

internal data class NotificationAppChoice(val packageName: String, val label: String)

/** Launchable apps, plus selected and previously seen packages. Names only. */
internal fun notificationAppChoices(
    context: Context,
    selected: Set<String>,
    observed: Set<String>,
): List<NotificationAppChoice> {
    val own = context.packageName
    val labels = linkedMapOf<String, String>()
    val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
    val activities = runCatching {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.packageManager.queryIntentActivities(launcher, PackageManager.ResolveInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.queryIntentActivities(launcher, 0)
        }
    }.getOrDefault(emptyList())
    for (info in activities) {
        val name = info.activityInfo?.packageName ?: continue
        if (name == own || !NotificationFilterPolicy.validPackage(name) || name in labels) continue
        labels[name] = appLabel(context, name)
    }
    for (name in (selected + observed).sorted()) {
        if (name == own || !NotificationFilterPolicy.validPackage(name) || name in labels) continue
        labels[name] = appLabel(context, name)
    }
    return labels.map { (name, label) -> NotificationAppChoice(name, label) }
        .sortedBy { it.label.lowercase() }
}

private fun appLabel(context: Context, packageName: String): String {
    val raw = runCatching {
        val info = context.packageManager.getApplicationInfo(packageName, 0)
        context.packageManager.getApplicationLabel(info).toString()
    }.getOrDefault(packageName)
    return raw.filterNot { it.isISOControl() }.take(64).ifBlank { packageName }
}

private fun save(
    store: NotificationFilterStore,
    next: NotificationFilter,
    onSaved: (NotificationFilter) -> Unit,
    onNotice: (String?) -> Unit,
) {
    if (store.save(next)) {
        onSaved(next)
        onNotice(null)
        NotificationListener.instance?.applyFilter()
    } else {
        onNotice("Could not save these choices. The previous selection is still in effect.")
    }
}

@Composable
private fun FilterSwitch(
    title: String,
    summary: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            colors = nectarSwitchColors(),
        )
    }
}

@Composable
private fun ModeRow(title: String, summary: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().selectable(selected = selected, role = Role.RadioButton, onClick = onClick),
    ) {
        RadioButton(selected = selected, onClick = onClick)
        Column(Modifier.padding(start = 8.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.45f))
}

private fun kindTitle(kind: NotificationKind): String = when (kind) {
    NotificationKind.MESSAGES -> "Messages"
    NotificationKind.EMAIL -> "Email"
    NotificationKind.OTHER -> "Other"
}

private fun kindSummary(kind: NotificationKind): String = when (kind) {
    NotificationKind.MESSAGES -> "Chats and other message-category notifications"
    NotificationKind.EMAIL -> "Mail-category notifications"
    NotificationKind.OTHER -> "Everything else this app already mirrors"
}

private fun selectedCount(count: Int): String = when (count) {
    0 -> "No apps selected"
    1 -> "1 app selected"
    else -> "$count apps selected"
}

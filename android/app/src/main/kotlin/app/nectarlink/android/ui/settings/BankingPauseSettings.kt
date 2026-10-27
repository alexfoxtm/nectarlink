// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import app.nectarlink.android.ui.settings.SettingsButton as Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import app.nectarlink.android.ui.settings.SettingsOutlinedButton as OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import app.nectarlink.android.ui.settings.SettingsTextButton as TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.banking.OpenWithPauseActivity
import app.nectarlink.android.elevated.Elevated

/** An app the user can choose: package, name and a small icon. */
internal data class Choosable(val pkg: String, val label: String, val icon: ImageBitmap?)

/** Settings › Control › Pause for banking apps. */
@Composable
internal fun BankingPauseAccess() {
    val context = LocalContext.current
    var chosen by remember { mutableStateOf(BankingPause.apps(context)) }
    var canChange by remember { mutableStateOf(BankingPause.canChangeSettings(context)) }
    var usage by remember { mutableStateOf(BankingPause.hasUsageAccess(context)) }
    var picking by remember { mutableStateOf(false) }
    val paused by BankingPause.paused.collectAsStateWithLifecycle()
    // Permissions can change in Settings while NectarLink is in the background.
    LifecycleResumeEffect(Unit) {
        canChange = BankingPause.canChangeSettings(context)
        usage = BankingPause.hasUsageAccess(context)
        onPauseOrDispose {}
    }

    val canPin = if (chosen.isNotEmpty()) remember { OpenWithPauseActivity.canPin(context) } else false
    val apps = if (chosen.isNotEmpty()) remember(chosen) { describe(context, chosen) } else emptyList()
    BankingPauseContent(
        chosenCount = chosen.size,
        canChange = canChange,
        usage = usage,
        paused = paused,
        canPin = canPin,
        apps = apps,
        onChooseApps = { picking = true },
        onAllowUsage = {
            // Through Elevated when it's running, else Android's own page.
            if (!Elevated.grantUsageAccess()) openUsageAccessSettings(context)
            usage = BankingPause.hasUsageAccess(context)
        },
        onResume = { BankingPause.resume(context) },
        onPin = { app -> OpenWithPauseActivity.pin(context, app.pkg, app.label) },
    )

    if (picking) {
        AppPicker(
            initial = chosen,
            onDone = { picked ->
                picking = false
                BankingPause.setApps(context, picked)
                chosen = picked
                // Elevated can let NectarLink see the app in front right away.
                if (picked.isNotEmpty() && !BankingPause.hasUsageAccess(context)) {
                    Elevated.grantUsageAccess()
                    usage = BankingPause.hasUsageAccess(context)
                }
            },
            onCancel = { picking = false },
        )
    }
}

/** Rendering only: app queries, permissions and banking actions stay in BankingPauseAccess. */
@Composable
internal fun BankingPauseContent(
    chosenCount: Int,
    canChange: Boolean,
    usage: Boolean,
    paused: Boolean,
    canPin: Boolean,
    apps: List<Choosable>,
    onChooseApps: () -> Unit,
    onAllowUsage: () -> Unit,
    onResume: () -> Unit,
    onPin: (Choosable) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        SettingsControlRow(
            title = stringResource(R.string.banking_title),
            supportingText = if (chosenCount == 0) {
                stringResource(R.string.banking_text)
            } else {
                pluralStringResource(R.plurals.banking_chosen, chosenCount, chosenCount)
            },
        ) {
            OutlinedButton(onClick = onChooseApps) { Text(stringResource(R.string.banking_choose)) }
        }

        if (chosenCount > 0) {
            when {
                !canChange -> Note(stringResource(R.string.banking_needs_elevated))
                !usage -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Note(stringResource(R.string.banking_needs_usage))
                    Button(onClick = onAllowUsage) { Text(stringResource(R.string.action_allow)) }
                }
                paused -> Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Note(stringResource(R.string.banking_paused_now))
                    TextButton(onClick = onResume) {
                        Text(stringResource(R.string.banking_turn_back_on))
                    }
                }
            }
            for (app in apps) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    AppIcon(app.icon)
                    Text(
                        app.label,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
                    )
                    if (canPin) {
                        TextButton(onClick = { onPin(app) }) {
                            Text(stringResource(R.string.banking_shortcut))
                        }
                    }
                }
            }
            if (canPin) Note(stringResource(R.string.banking_shortcut_hint))
        }
    }
}

@Composable
private fun AppPicker(initial: Set<String>, onDone: (Set<String>) -> Unit, onCancel: () -> Unit) {
    val context = LocalContext.current
    val apps = remember { launchableApps(context) }
    var selected by remember { mutableStateOf(initial) }
    var query by remember { mutableStateOf("") }
    val shown = apps.filter { query.isBlank() || it.label.contains(query.trim(), ignoreCase = true) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(stringResource(R.string.banking_pick_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    label = { Text(stringResource(R.string.banking_search)) },
                    modifier = Modifier.fillMaxWidth(),
                )
                LazyColumn(Modifier.heightIn(max = 360.dp)) {
                    items(shown, key = { it.pkg }) { app ->
                        val on = app.pkg in selected
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { selected = if (on) selected - app.pkg else selected + app.pkg }
                                .padding(vertical = 4.dp),
                        ) {
                            Checkbox(checked = on, onCheckedChange = null)
                            AppIcon(app.icon)
                            Text(app.label, modifier = Modifier.padding(start = 12.dp))
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onDone(selected) }) { Text(stringResource(R.string.action_done)) } },
        dismissButton = { TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) } },
    )
}

@Composable
private fun AppIcon(icon: ImageBitmap?) {
    if (icon != null) Image(icon, contentDescription = null, modifier = Modifier.size(28.dp))
}

@Composable
private fun Note(text: String, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = modifier)
}

/** Apps with a launcher icon, by name, NectarLink left out. */
private fun launchableApps(context: Context): List<Choosable> {
    val manager = context.packageManager
    val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
    @Suppress("DEPRECATION")
    val found = manager.queryIntentActivities(launcher, 0)
    return found.map { it.activityInfo }
        .filter { it.packageName != context.packageName }
        .distinctBy { it.packageName }
        .map { info -> Choosable(info.packageName, info.loadLabel(manager).toString().trim(), iconOf(manager, info.packageName)) }
        .sortedBy { it.label.lowercase() }
}

/** The chosen apps that are still installed, by name. */
private fun describe(context: Context, packages: Set<String>): List<Choosable> {
    val manager = context.packageManager
    return packages.mapNotNull { pkg ->
        runCatching {
            val info = manager.getApplicationInfo(pkg, 0)
            Choosable(pkg, manager.getApplicationLabel(info).toString(), iconOf(manager, pkg))
        }.getOrNull()
    }.sortedBy { it.label.lowercase() }
}

private fun iconOf(manager: PackageManager, pkg: String): ImageBitmap? =
    runCatching { manager.getApplicationIcon(pkg).toBitmap(96, 96).asImageBitmap() }.getOrNull()

private fun openUsageAccessSettings(context: Context) {
    runCatching {
        context.startActivity(Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

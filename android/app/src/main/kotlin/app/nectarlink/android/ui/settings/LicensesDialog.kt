// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import android.content.Intent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.net.toUri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import app.nectarlink.android.ui.theme.glassModalContainerColor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The license texts and notices packaged under `assets/licenses` by the build. */
private const val LICENSES_DIR = "licenses"
private const val FORK_REPOSITORY = "https://github.com/alexfoxtm/nectarlink"

/** Index first, then the app's own license table, then the rest by name. */
private val LEADING = listOf("THIRD-PARTY-NOTICES.txt", "LICENSE.txt")

/** About › Open-source licenses: the license statement, this build's source, and every bundled text. */
@Composable
internal fun LicensesDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val documents = remember {
        context.assets.list(LICENSES_DIR).orEmpty()
            .sortedWith(compareBy<String> { LEADING.indexOf(it).let { i -> if (i < 0) LEADING.size else i } }.thenBy { it })
    }
    var open by remember { mutableStateOf<String?>(null) }
    val openUrl = { url: String -> runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, url.toUri())) } }

    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = glassModalContainerColor(),
        title = { Text(stringResource(R.string.about_licenses_title)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Text(stringResource(R.string.about_licenses_text))
                Text(
                    stringResource(R.string.about_source_title),
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.padding(top = 16.dp),
                )
                if (BuildConfig.SOURCE_URL_IS_EXACT) {
                    TextButton(onClick = { openUrl(BuildConfig.SOURCE_URL) }) {
                        Text(stringResource(R.string.about_source_build))
                    }
                    SelectionContainer {
                        Text(
                            BuildConfig.SOURCE_URL,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else {
                    Text(
                        stringResource(R.string.about_source_unpublished),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                TextButton(onClick = { openUrl(FORK_REPOSITORY) }) {
                    Text(stringResource(R.string.about_source_fork))
                }
                Text(
                    stringResource(R.string.about_licenses_documents),
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.padding(top = 16.dp),
                )
                documents.forEach { name ->
                    TextButton(onClick = { open = name }) { Text(name) }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_done)) }
        },
    )

    open?.let { name -> LicenseDocument(name, onDismiss = { open = null }) }
}

/** One bundled text, full screen. Notices run to hundreds of KB, so paragraphs are laid out lazily. */
@Composable
private fun LicenseDocument(name: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val paragraphs by produceState<List<String>?>(initialValue = null, name) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                context.assets.open("$LICENSES_DIR/$name").bufferedReader().use { it.readText() }.split("\n\n")
            }.getOrElse { emptyList() }
        }
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize(), color = glassModalContainerColor()) {
            Column(Modifier.fillMaxSize().padding(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text(name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                    TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_done)) }
                }
                val loaded = paragraphs
                when {
                    loaded == null -> Unit
                    loaded.isEmpty() -> Text(stringResource(R.string.about_license_unavailable))
                    else -> LazyColumn(Modifier.weight(1f)) {
                        items(loaded) { paragraph ->
                            Text(
                                paragraph,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                modifier = Modifier.padding(bottom = 12.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

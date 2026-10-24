// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.addCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.security.MessageDigest
import java.util.UUID

internal const val RECEIVED_APK_MIME = "application/vnd.android.package-archive"
private const val STAGED_UPDATES = "verified-updates"
private const val MAX_APK_BYTES = 300L * 1024 * 1024

/** Exact current signers only; another package, unsigned archive, or signer rotation is not admitted. */
internal fun isVerifiedSelfPackage(
    installedPackage: String,
    candidatePackage: String?,
    installedSigners: Set<String>,
    candidateSigners: Set<String>,
): Boolean = candidatePackage == installedPackage && installedSigners.isNotEmpty() && installedSigners == candidateSigners

/** The same attempt owns retirement, handoff and any failed/cancelled handoff recovery. */
internal suspend fun launchVerifiedReceivedUpdate(
    attempt: String,
    retire: suspend (String) -> Boolean,
    launch: () -> Unit,
    resume: (String) -> Unit,
): Boolean {
    var handedOff = false
    try {
        if (!retire(attempt)) return false
        launch()
        handedOff = true
        return true
    } finally {
        if (!handedOff) resume(attempt)
    }
}

@Suppress("DEPRECATION")
private fun signerDigests(info: PackageInfo): Set<String> {
    val signers = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) info.signingInfo?.apkContentsSigners else info.signatures
    return signers.orEmpty().map { signer ->
        MessageDigest.getInstance("SHA-256").digest(signer.toByteArray()).joinToString("") { "%02x".format(it) }
    }.toSet()
}

/** Inspect private bytes and hand exactly those bytes to the installer; the Downloads URI is mutable. */
@Suppress("DEPRECATION")
private suspend fun stageVerifiedSelfApk(context: Context, source: Uri, staged: File): Boolean {
    var keep = false
    try {
        staged.parentFile?.mkdirs()
        val input = context.contentResolver.openInputStream(source) ?: return false
        input.use { from ->
            staged.outputStream().use { to ->
                val bytes = ByteArray(64 * 1024)
                var total = 0L
                while (true) {
                    currentCoroutineContext().ensureActive()
                    val count = from.read(bytes)
                    if (count < 0) break
                    total += count
                    if (total > MAX_APK_BYTES) return false
                    to.write(bytes, 0, count)
                }
            }
        }
        currentCoroutineContext().ensureActive()
        val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
        val manager = context.packageManager
        val installed = manager.getPackageInfo(context.packageName, flags)
        val candidate = manager.getPackageArchiveInfo(staged.absolutePath, flags) ?: return false
        if (!isVerifiedSelfPackage(context.packageName, candidate.packageName, signerDigests(installed), signerDigests(candidate))) return false
        currentCoroutineContext().ensureActive()
        keep = true
        return true
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Exception) {
        return false
    } finally {
        if (!keep) runCatching { staged.delete() }
    }
}

/** Internal, user-tapped APK open route; it neither installs nor grants permission automatically. */
class ReceivedSelfUpdateActivity : ComponentActivity() {
    private lateinit var attempt: String
    private var stagedName: String? = null
    private var waitingForInstaller = false
    private val core get() = (applicationContext as NectarlinkApplication).core
    private val installer = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        if (result.resultCode == android.app.Activity.RESULT_OK) {
            // A successful replacement must not start another old-process Node
            // in the gap before Android finishes replacing this process.
            waitingForInstaller = false
            deleteStaged()
            finish()
        } else {
            finishAttempt()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        waitingForInstaller = savedInstanceState?.getBoolean("waiting") == true
        // Only a handed-off installer retains its attempt across recreation. A
        // cancelled pre-handoff coroutine must not release the next attempt.
        attempt = if (waitingForInstaller) savedInstanceState?.getString("attempt")
            ?: UUID.randomUUID().toString() else UUID.randomUUID().toString()
        stagedName = if (waitingForInstaller) savedInstanceState?.getString("staged") else null
        onBackPressedDispatcher.addCallback(this) { finishAttempt() }
        if (waitingForInstaller) return
        val source = intent.data ?: return finish()
        lifecycleScope.launch {
            var handedOff = false
            try {
                stagedName = UUID.randomUUID().toString() + ".apk"
                val staged = File(File(cacheDir, STAGED_UPDATES), stagedName!!)
                val verified = withContext(Dispatchers.IO) { stageVerifiedSelfApk(this@ReceivedSelfUpdateActivity, source, staged) }
                if (!verified) {
                    val view = Intent(Intent.ACTION_VIEW).setDataAndType(source, RECEIVED_APK_MIME)
                        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    startActivity(Intent.createChooser(view, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    return@launch
                }
                if (!lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) return@launch
                val uri = FileProvider.getUriForFile(this@ReceivedSelfUpdateActivity, packageName + ".files", staged)
                @Suppress("DEPRECATION")
                val install = Intent(Intent.ACTION_INSTALL_PACKAGE).setDataAndType(uri, RECEIVED_APK_MIME)
                    .putExtra(Intent.EXTRA_RETURN_RESULT, true)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                handedOff = launchVerifiedReceivedUpdate(
                    attempt,
                    retire = { core.shutdownBeforeVerifiedUpdate(it) },
                    launch = {
                        check(!isFinishing && lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                        waitingForInstaller = true
                        installer.launch(install)
                    },
                    resume = { core.resumeAfterVerifiedUpdateCancelled(it) },
                )
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // A failed handoff never falls through to an unretired self-install.
            } finally {
                if (!handedOff) {
                    deleteStaged()
                    if (!isChangingConfigurations) finish()
                }
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("attempt", attempt)
        outState.putString("staged", stagedName)
        outState.putBoolean("waiting", waitingForInstaller)
        super.onSaveInstanceState(outState)
    }

    private fun finishAttempt() {
        waitingForInstaller = false
        core.resumeAfterVerifiedUpdateCancelled(attempt)
        deleteStaged()
        finish()
    }

    private fun deleteStaged() {
        val name = stagedName ?: return
        stagedName = null
        // Only the private copy made for this attempt; never delete the received Downloads file.
        if (File(name).name == name) runCatching { File(File(cacheDir, STAGED_UPDATES), name).delete() }
    }
}

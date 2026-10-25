// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.app.LocaleManager
import android.content.Context
import android.os.Build
import android.os.LocaleList
import androidx.annotation.RequiresApi

/**
 * The app's own language, kept by Android (13 and newer) as the per-app language: it shows
 * in the system's App languages screen too, and applies everywhere the app shows text,
 * notifications and background services included. Older Android follows the phone.
 */
object AppLanguage {
    /** The languages the app speaks, as BCP 47 tags, each named in its own language. */
    val languages: List<Pair<String, String>> = listOf(
        "en" to "English",
    )

    /** Whether the phone lets the app pick its own language. */
    val supported: Boolean
        get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

    /** The picked language's tag, or "" to follow the phone. */
    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    fun current(context: Context): String {
        val locales = context.getSystemService(LocaleManager::class.java)?.applicationLocales ?: return ""
        return if (locales.isEmpty) "" else normalize(locales[0].toLanguageTag())
    }

    /** Switches to [tag] ("" for the phone's). Android restarts the open screens in it. */
    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    fun set(context: Context, tag: String) {
        val manager = context.getSystemService(LocaleManager::class.java) ?: return
        manager.applicationLocales = if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
    }

    /**
     * One of [languages]' tags for [tag], or "" when the app doesn't speak it. Android may
     * hand back a regional tag ("en-US") for an offered language.
     */
    internal fun normalize(tag: String): String {
        languages.firstOrNull { it.first.equals(tag, ignoreCase = true) }?.let { return it.first }
        val lower = tag.lowercase()
        return when {
            lower == "en" || lower.startsWith("en-") -> "en"
            else -> ""
        }
    }
}

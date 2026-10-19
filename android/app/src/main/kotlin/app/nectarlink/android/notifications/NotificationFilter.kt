// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import android.app.Notification
import android.content.Context
import android.content.SharedPreferences

/**
 * Phone-side ceiling for which notifications may be mirrored.
 *
 * The choice (every app, or an explicit set; message, email, and other)
 * follows the local Waypair notification-mirror policy. It was rewritten
 * for this GPL tree rather than copied from that Apache-2.0 source. The
 * default mirrors whatever this app already mirrors. It does not read or
 * log notification text, and it does not change SMS permission. This app
 * has no Gmail code reader. A code the computer copies comes from a
 * notification that passed this filter.
 */
internal enum class NotificationAppMode { ALL, SELECTED }

internal enum class NotificationKind { MESSAGES, EMAIL, OTHER }

internal data class NotificationFilter(
    val enabled: Boolean = true,
    val appMode: NotificationAppMode = NotificationAppMode.ALL,
    val selectedPackages: Set<String> = emptySet(),
    val kinds: Set<NotificationKind> = NotificationKind.entries.toSet(),
) {
    /** App and type choice only. Structural skips are [NotificationFilterPolicy.forwardable]. */
    fun allows(sourcePackage: String, kind: NotificationKind, currentUser: Boolean = true): Boolean {
        if (!enabled || !currentUser || !NotificationFilterPolicy.validPackage(sourcePackage)) return false
        if (kind !in kinds) return false
        return appMode == NotificationAppMode.ALL || sourcePackage in selectedPackages
    }

    companion object {
        /** A record we cannot trust. Mirrors nothing, so a corrupt file cannot widen sharing. */
        val BLOCKED = NotificationFilter(
            enabled = false,
            appMode = NotificationAppMode.SELECTED,
            selectedPackages = emptySet(),
            kinds = emptySet(),
        )
    }
}

internal object NotificationFilterPolicy {
    const val MAX_PACKAGES = 256
    private const val MAX_PACKAGE_BYTES = 255
    private val PACKAGE = Regex("[A-Za-z_][A-Za-z0-9_]*(\\.[A-Za-z_][A-Za-z0-9_]*)*")

    fun validPackage(sourcePackage: String): Boolean =
        sourcePackage.toByteArray(Charsets.UTF_8).size in 1..MAX_PACKAGE_BYTES && PACKAGE.matches(sourcePackage)

    /**
     * Android category values (`msg`, `email`). A messaging-style extra wins
     * over email, matching the usual notification-category order. No body is read.
     */
    fun notificationKind(category: String?, template: String?, hasMessages: Boolean): NotificationKind = when {
        category == "msg" || hasMessages -> NotificationKind.MESSAGES
        category == "email" || template?.endsWith("InboxStyle") == true -> NotificationKind.EMAIL
        else -> NotificationKind.OTHER
    }

    fun kindOf(notification: Notification): NotificationKind {
        val extras = notification.extras
        return notificationKind(
            category = notification.category,
            template = extras?.getString(Notification.EXTRA_TEMPLATE),
            hasMessages = extras?.containsKey(Notification.EXTRA_MESSAGES) == true,
        )
    }

    /**
     * Whether this notification may be read and forwarded. Ongoing and
     * foreground-service notifications are left to [NotificationReader], which
     * already keeps live updates and drops the rest. Group summaries, local-only
     * notifications, another Android user, and this app itself never pass.
     */
    fun forwardable(
        ownPackage: String,
        sourcePackage: String,
        currentUser: Boolean,
        groupSummary: Boolean,
        localOnly: Boolean,
        kind: NotificationKind,
        filter: NotificationFilter,
    ): Boolean {
        if (!currentUser || groupSummary || localOnly || sourcePackage == ownPackage) return false
        return filter.allows(sourcePackage, kind, currentUser = true)
    }

    /** Missing text is the open default. Anything we cannot parse is [NotificationFilter.BLOCKED]. */
    fun decode(raw: String?): NotificationFilter {
        if (raw.isNullOrEmpty()) return NotificationFilter()
        if (raw.length > MAX_RECORD) return NotificationFilter.BLOCKED
        val parts = raw.split('|')
        if (parts.size != 5 || parts[0] != "1") return NotificationFilter.BLOCKED
        val enabled = when (parts[1]) {
            "true" -> true
            "false" -> false
            else -> return NotificationFilter.BLOCKED
        }
        val mode = when (parts[2]) {
            "ALL" -> NotificationAppMode.ALL
            "SELECTED" -> NotificationAppMode.SELECTED
            else -> return NotificationFilter.BLOCKED
        }
        val packages = if (parts[3].isEmpty()) emptySet() else parts[3].split(',').toSet()
        if (packages.size > MAX_PACKAGES || packages.any { !validPackage(it) }) return NotificationFilter.BLOCKED
        val kindNames = if (parts[4].isEmpty()) emptyList() else parts[4].split(',')
        val kinds = kindNames.map { name ->
            NotificationKind.entries.firstOrNull { it.name == name } ?: return NotificationFilter.BLOCKED
        }.toSet()
        return NotificationFilter(enabled, mode, packages, kinds)
    }

    fun encode(filter: NotificationFilter): String? {
        if (filter.selectedPackages.size > MAX_PACKAGES || filter.selectedPackages.any { !validPackage(it) }) return null
        val packages = filter.selectedPackages.sorted().joinToString(",")
        val kinds = filter.kinds.sortedBy { it.ordinal }.joinToString(",") { it.name }
        val raw = listOf(
            "1",
            filter.enabled.toString(),
            filter.appMode.name,
            packages,
            kinds,
        ).joinToString("|")
        return raw.takeIf { it.length <= MAX_RECORD }
    }

    private const val MAX_RECORD = 8_192
}

/**
 * One local record. Package names only: never a title, body, or code.
 * Absent preferences keep today's mirroring. A corrupt record blocks mirroring.
 */
internal class NotificationFilterStore(private val prefs: SharedPreferences) {
    fun load(): NotificationFilter = runCatching {
        NotificationFilterPolicy.decode(prefs.getString(KEY_FILTER, null))
    }.getOrDefault(NotificationFilter.BLOCKED)

    fun save(filter: NotificationFilter): Boolean {
        val encoded = NotificationFilterPolicy.encode(filter) ?: return false
        return prefs.edit().putString(KEY_FILTER, encoded).commit()
    }

    /** Package names seen or saved. Read from preferences so every store sees the same set. */
    fun observed(): Set<String> = prefs.getStringSet(KEY_OBSERVED, emptySet()).orEmpty()
        .filter(NotificationFilterPolicy::validPackage)
        .take(MAX_OBSERVED)
        .toSet()

    fun notePackage(name: String, ownPackage: String) {
        if (name == ownPackage || !NotificationFilterPolicy.validPackage(name)) return
        synchronized(NotificationFilterStore::class.java) {
            val next = observed().toMutableSet()
            if (name in next || next.size >= MAX_OBSERVED) return
            next.add(name)
            prefs.edit().putStringSet(KEY_OBSERVED, next).apply()
        }
    }

    companion object {
        private const val PREFS = "notification-filter"
        private const val KEY_FILTER = "filter"
        private const val KEY_OBSERVED = "observed"
        private const val MAX_OBSERVED = 256

        fun create(context: Context): NotificationFilterStore = NotificationFilterStore(
            context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE),
        )
    }
}

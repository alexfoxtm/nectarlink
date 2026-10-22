// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import kotlin.math.max
import kotlin.math.min

/** Captured width/height are already oriented: never transpose them a second time. */
internal fun fitCapturedContent(width: Int, height: Int, maxSize: Int, dpi: Int): Triple<Int, Int, Int>? {
    if (width <= 0 || height <= 0 || maxSize < 16 || dpi <= 0) return null
    val scale = min(1.0, maxSize.toDouble() / max(width, height))
    fun fit(value: Int) = max(16, ((value * scale) / 16).toInt() * 16)
    return Triple(fit(width), fit(height), dpi)
}

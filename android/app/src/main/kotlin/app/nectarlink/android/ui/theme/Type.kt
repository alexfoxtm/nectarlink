// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import android.content.res.AssetManager
import androidx.compose.material3.Typography
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight

/** Independently bundled OFL Inter; no reference-application font assets are used. */
@Immutable
class AppFonts(assets: AssetManager) {
    val sans = FontFamily(
        listOf(400, 500, 600, 650, 700).map { weight ->
            Font(
                resId = app.nectarlink.android.R.font.inter_variable,
                weight = FontWeight(weight),
                variationSettings = FontVariation.Settings(FontVariation.weight(weight)),
            )
        },
    )
    // Retained API for presentation call sites; headings use the same native sans family.
    val serif = sans
    val mono = FontFamily(
        Font("fonts/SpaceMono-Regular.ttf", assets),
        Font("fonts/SpaceMono-Bold.ttf", assets, FontWeight.Bold),
    )
}

val LocalAppFonts = staticCompositionLocalOf<AppFonts> { error("NectarlinkTheme provides the fonts") }

/** Explicit Inter scale from the R69 factual contract, with Android font scaling intact. */
internal fun typography(fonts: AppFonts, @Suppress("UNUSED_PARAMETER") graphite: Boolean): Typography {
    fun text(size: Int, line: Int, weight: Int, tracking: Float) = TextStyle(
        fontFamily = fonts.sans,
        fontWeight = FontWeight(weight),
        fontSize = size.sp,
        lineHeight = line.sp,
        letterSpacing = tracking.sp,
    )
    val display = text(44, 52, 600, -0.92f)
    val page = text(28, 34, 600, -0.37f)
    val heading = text(20, 24, 600, -0.12f)
    val body = text(16, 20, 500, 0.02f)
    val supporting = text(14, 18, 500, 0.07f)
    val caption = text(12, 16, 500, 0.18f)
    return Typography(
        displayLarge = display, displayMedium = display, displaySmall = page,
        headlineLarge = page, headlineMedium = page, headlineSmall = heading,
        titleLarge = heading, titleMedium = body, titleSmall = body,
        bodyLarge = body, bodyMedium = supporting, bodySmall = supporting,
        labelLarge = body, labelMedium = supporting, labelSmall = caption,
    )
}

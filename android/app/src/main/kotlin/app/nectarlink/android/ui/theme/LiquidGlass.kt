// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import android.graphics.RenderEffect
import android.graphics.Shader
import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asComposeRenderEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The content that floating glass controls sit over. The source records what it draws
 * into [layer]; glass surfaces redraw that recording, blurred behind the floating control.
 */
@Stable
class GlassBackdrop internal constructor(internal val layer: GraphicsLayer) {
    internal var origin by mutableStateOf(Offset.Zero)
    internal var recorded by mutableStateOf(false)
}

@Composable
fun rememberGlassBackdrop(): GlassBackdrop {
    val layer = rememberGraphicsLayer()
    return remember(layer) { GlassBackdrop(layer) }
}

/** Marks the content that [liquidGlass] surfaces sample. Drawing is unchanged. */
fun Modifier.glassBackdropSource(backdrop: GlassBackdrop, enabled: Boolean): Modifier =
    if (!enabled || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
        this
    } else {
        this
            .onGloballyPositioned { backdrop.origin = it.positionInRoot() }
            .drawWithContent {
                val captured = runCatching {
                    backdrop.layer.record { this@drawWithContent.drawContent() }
                    drawLayer(backdrop.layer)
                }.isSuccess
                backdrop.recorded = captured
                if (!captured) drawContent()
            }
    }

/**
 * A floating Liquid Glass surface: the backdrop behind it, lightly frosted
 * with a quiet edge and a tint that keeps text readable.
 * Falls back to [fallbackColor] where the shader isn't available or [enabled] is false.
 */
@Composable
fun Modifier.liquidGlass(
    backdrop: GlassBackdrop?,
    enabled: Boolean,
    cornerRadius: Dp,
    tint: Color,
    fallbackColor: Color,
    rimColor: Color,
    dark: Boolean,
): Modifier {
    val hardware = LocalView.current.isHardwareAccelerated
    val origin = remember { mutableStateOf(Offset.Zero) }
    return this
        .onGloballyPositioned { origin.value = it.positionInRoot() }
        .drawWithCache {
            val r = cornerRadius.toPx().coerceAtMost(size.minDimension / 2f)
            val cr = CornerRadius(r, r)
            val path = Path().apply { addRoundRect(RoundRect(0f, 0f, size.width, size.height, cr)) }
            val stroke = Stroke(width = 1.dp.toPx())
            val glassLayer = if (enabled && hardware && backdrop != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                runCatching { obtainGraphicsLayer().also {
                    it.renderEffect = RenderEffect.createBlurEffect(
                        12.dp.toPx(), 12.dp.toPx(), Shader.TileMode.CLAMP,
                    ).asComposeRenderEffect()
                } }.getOrNull()
            } else {
                null
            }
            val rim = rimColor.copy(alpha = if (dark) 0.10f else 0.12f)
            onDrawBehind {
                val sampled = if (glassLayer != null && backdrop != null && backdrop.recorded) {
                    runCatching {
                        val dx = backdrop.origin.x - origin.value.x
                        val dy = backdrop.origin.y - origin.value.y
                        glassLayer.record { translate(dx, dy) { drawLayer(backdrop.layer) } }
                        clipPath(path) { drawLayer(glassLayer) }
                    }.isSuccess
                } else false
                drawPath(path, if (sampled) tint else fallbackColor)
                drawPath(path, rim, style = stroke)
            }
        }
}

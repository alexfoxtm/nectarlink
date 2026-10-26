// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import androidx.annotation.DrawableRes
import androidx.compose.foundation.Canvas
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.res.painterResource
import app.nectarlink.android.R

/** Owned Link glyphs share semantic inks; caller status colors and disabled alpha survive. */
@Composable
fun LinkIcon(
    @DrawableRes resourceId: Int,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    tint: Color = LocalContentColor.current,
    colorize: Boolean = true,
) {
    Icon(
        painter = painterResource(resourceId),
        contentDescription = contentDescription,
        modifier = modifier,
        tint = if (colorize) linkIconColor(resourceId, tint) else tint,
    )
}

/** Choose ink from the actual foreground, including neutral light/dark fallback surfaces. */
fun linkIconColor(@DrawableRes resourceId: Int, fallback: Color): Color {
    if (fallback == Color.Unspecified || fallback.alpha < 0.6f) return fallback
    val chroma = maxOf(fallback.red, fallback.green, fallback.blue) - minOf(fallback.red, fallback.green, fallback.blue)
    if (chroma > 0.08f) return fallback
    val bright = fallback.luminance() >= 0.18f
    val ink = when (resourceId) {
        R.drawable.wp_monitor, R.drawable.wp_oneui_device_outline, R.drawable.wp_wifi,
        R.drawable.wp_qr, R.drawable.wp_link_search, R.drawable.wp_control_color,
        R.drawable.wp_home, R.drawable.wp_home_filled -> if (bright) Color(0xFF73A8FF) else Color(0xFF2463BD)
        R.drawable.wp_folder_color, R.drawable.wp_browse_color, R.drawable.wp_send_color,
        R.drawable.wp_clipboard_color, R.drawable.ic_clipboard -> if (bright) Color(0xFF61C9A0) else Color(0xFF187543)
        R.drawable.wp_bell_color, R.drawable.wp_sun, R.drawable.wp_battery,
        R.drawable.wp_oneui_lightning_outline, R.drawable.wp_oneui_star_outline -> if (bright) Color(0xFFE9B65A) else Color(0xFF996000)
        R.drawable.wp_camera_color, R.drawable.wp_mic_color, R.drawable.ic_music,
        R.drawable.ic_play, R.drawable.ic_pause, R.drawable.ic_skip_next, R.drawable.ic_skip_previous,
        R.drawable.ic_speaker, R.drawable.ic_sound_off, R.drawable.wp_deck_color,
        R.drawable.wp_oneui_mic_off, R.drawable.wp_oneui_video_call_outline,
        R.drawable.wp_oneui_volume_up, R.drawable.wp_oneui_volume_down -> if (bright) Color(0xFFBC9BF2) else Color(0xFF8052BD)
        R.drawable.wp_settings, R.drawable.wp_settings_filled, R.drawable.ic_settings,
        R.drawable.ic_refresh, R.drawable.wp_globe, R.drawable.wp_moon, R.drawable.wp_clock_color,
        R.drawable.wp_oneui_apps_outline, R.drawable.wp_oneui_keyboard, R.drawable.wp_oneui_keyboard_btn_delete,
        R.drawable.wp_oneui_bracket, R.drawable.wp_oneui_font_size, R.drawable.wp_oneui_lock_outline,
        R.drawable.wp_oneui_security_outline, R.drawable.wp_oneui_system_alert_window,
        R.drawable.wp_oneui_power, R.drawable.wp_oneui_arrow_left, R.drawable.wp_oneui_arrow_right,
        R.drawable.wp_oneui_arrow_up, R.drawable.wp_oneui_arrow_down -> if (bright) Color(0xFF8EAADC) else Color(0xFF4B6E98)
        else -> return fallback
    }
    return ink.copy(alpha = fallback.alpha)
}

/** Shared role for an existing successful/online state; selection checks keep their own ink. */
fun linkSuccessColor(fallback: Color): Color = linkIconColor(R.drawable.wp_clipboard_color, fallback)

/** Original native content illustration, contained within the existing device-avatar bounds. */
@Composable
fun LinkDeviceGraphic(modifier: Modifier = Modifier) {
    val edge = linkIconColor(R.drawable.wp_monitor, MaterialTheme.colorScheme.onSurface)
    val phoneEdge = linkIconColor(R.drawable.wp_camera_color, MaterialTheme.colorScheme.onSurface)
    val panel = MaterialTheme.colorScheme.surface
    Canvas(modifier) {
        val w = size.width
        val h = size.height
        drawRoundRect(edge, Offset(w * 0.05f, h * 0.10f), Size(w * 0.90f, h * 0.61f), CornerRadius(w * 0.09f))
        drawRoundRect(
            brush = Brush.linearGradient(listOf(Color(0xFF48BFE8), Color(0xFF327CE0), Color(0xFF9B72D8))),
            topLeft = Offset(w * 0.12f, h * 0.17f), size = Size(w * 0.76f, h * 0.43f), cornerRadius = CornerRadius(w * 0.04f),
        )
        drawRoundRect(edge, Offset(w * 0.43f, h * 0.68f), Size(w * 0.14f, h * 0.16f), CornerRadius(w * 0.03f))
        drawRoundRect(edge, Offset(w * 0.28f, h * 0.82f), Size(w * 0.44f, h * 0.06f), CornerRadius(w * 0.03f))
        drawRoundRect(phoneEdge, Offset(w * 0.02f, h * 0.43f), Size(w * 0.29f, h * 0.51f), CornerRadius(w * 0.06f))
        drawRoundRect(panel, Offset(w * 0.065f, h * 0.485f), Size(w * 0.20f, h * 0.37f), CornerRadius(w * 0.025f))
        drawRoundRect(
            brush = Brush.verticalGradient(listOf(Color(0xFF56C4D7), Color(0xFF7C83DF))),
            topLeft = Offset(w * 0.085f, h * 0.515f), size = Size(w * 0.16f, h * 0.27f), cornerRadius = CornerRadius(w * 0.018f),
        )
    }
}

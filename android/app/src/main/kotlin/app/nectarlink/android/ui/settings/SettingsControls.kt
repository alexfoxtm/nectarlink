// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LocalMinimumInteractiveComponentSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.Text
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import app.nectarlink.android.R
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Settings has a compact type hierarchy; the rest of the app keeps its own scale. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SettingsPresentation(content: @Composable () -> Unit) {
    val type = MaterialTheme.typography
    val title = type.titleSmall.copy(fontWeight = FontWeight.SemiBold, fontSize = 16.sp, lineHeight = 20.sp)
    val supporting = type.bodyMedium.copy(fontWeight = FontWeight.Normal, fontSize = 14.sp, lineHeight = 20.sp)
    val label = type.labelMedium.copy(fontWeight = FontWeight.Medium, fontSize = 14.sp, lineHeight = 20.sp)
    CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides 48.dp) {
        MaterialTheme(
            typography = type.copy(
                titleMedium = title, titleSmall = title,
                bodyMedium = supporting, bodySmall = supporting,
                labelLarge = label, labelMedium = label,
            ),
            content = content,
        )
    }
}

/** Long explanations keep the whole card width, independent of the trailing control. */
@Composable
internal fun SettingsControlRow(
    title: String,
    supportingText: String,
    control: @Composable RowScope.() -> Unit,
) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val stacked = maxWidth < 280.dp || LocalDensity.current.fontScale >= 1.3f
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            if (stacked) {
                Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.fillMaxWidth())
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    control()
                }
            } else {
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                    control()
                }
            }
            Text(
                supportingText,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/** Only the visible switch shrinks. One unscaled native toggle target owns all input. */
@Composable
internal fun SettingsSwitch(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: SwitchColors,
) {
    val interactions = remember { MutableInteractionSource() }
    Box(
        modifier = modifier
            .sizeIn(minWidth = 48.dp, minHeight = 48.dp)
            .toggleable(
                value = checked,
                enabled = enabled,
                role = Role.Switch,
                interactionSource = interactions,
                indication = ripple(bounded = false, radius = 24.dp),
                onValueChange = onCheckedChange,
            ),
        contentAlignment = Alignment.Center,
    ) {
        androidx.compose.material3.Switch(
            checked = checked,
            onCheckedChange = null,
            enabled = enabled,
            colors = colors,
            interactionSource = interactions,
            modifier = Modifier.requiredSize(52.dp, 32.dp)
                .graphicsLayer { scaleX = 44f / 52f; scaleY = 28f / 32f }
                .clearAndSetSemantics {},
        )
    }
}

@Composable
internal fun SettingsButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.Button(
    onClick = onClick, modifier = modifier, enabled = enabled, elevation = null,
    shape = RoundedCornerShape(50),
    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
    content = content,
)

@Composable
internal fun SettingsFilledTonalButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.FilledTonalButton(
    onClick = onClick, modifier = modifier, enabled = enabled, elevation = null,
    shape = RoundedCornerShape(50),
    colors = ButtonDefaults.filledTonalButtonColors(
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        contentColor = MaterialTheme.colorScheme.onSurface,
    ),
    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
    content = content,
)

@Composable
internal fun SettingsOutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.OutlinedButton(
    onClick = onClick, modifier = modifier, enabled = enabled,
    shape = RoundedCornerShape(50),
    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
    content = content,
)

@Composable
internal fun SettingsTextButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.TextButton(
    onClick = onClick, modifier = modifier, enabled = enabled,
    shape = RoundedCornerShape(50),
    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
    content = content,
)


internal enum class SettingsServiceAction { SET_UP, TRY_AGAIN, TURN_OFF, NONE }

/** Rendering only: service state and privileged actions belong to the production caller. */
@Composable
internal fun SettingsAppsServiceContent(
    statusText: String,
    action: SettingsServiceAction,
    showDeveloper: Boolean,
    rootOffered: Boolean,
    onSetUp: () -> Unit,
    onTryAgain: () -> Unit,
    onTurnOff: () -> Unit,
    onOpenDeveloper: () -> Unit,
    onUseRoot: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
        ) {
            Text(
                stringResource(R.string.settings_apps_service_status),
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier.weight(1f),
            )
            when (action) {
                SettingsServiceAction.SET_UP -> SettingsButton(onClick = onSetUp) {
                    Text(stringResource(R.string.elevated_set_up))
                }
                SettingsServiceAction.TRY_AGAIN -> SettingsButton(onClick = onTryAgain) {
                    Text(stringResource(R.string.elevated_try_again))
                }
                SettingsServiceAction.TURN_OFF -> SettingsOutlinedButton(onClick = onTurnOff) {
                    Text(stringResource(R.string.elevated_turn_off))
                }
                SettingsServiceAction.NONE -> {}
            }
        }
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Text(
                statusText,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            )
        }
        if (showDeveloper) {
            SettingsOutlinedButton(onClick = onOpenDeveloper) {
                Text(stringResource(R.string.elevated_open_developer))
            }
        }
        if (rootOffered) {
            Text(
                stringResource(R.string.elevated_root_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SettingsOutlinedButton(onClick = onUseRoot) {
                Text(stringResource(R.string.elevated_use_root))
            }
        }
    }
}

/** Rendering only: opening Android Settings remains owned by ControlAccess. */
@Composable
internal fun SettingsControlAccessContent(
    on: Boolean,
    showRestricted: Boolean,
    onOpenSettings: () -> Unit,
    onOpenAppInfo: () -> Unit,
) {
    SettingsControlRow(
        title = stringResource(R.string.control_title),
        supportingText = stringResource(if (on) R.string.control_on else R.string.control_off),
    ) {
        if (on) {
            SettingsOutlinedButton(onClick = onOpenSettings) { Text(stringResource(R.string.action_manage)) }
        } else {
            SettingsButton(onClick = onOpenSettings) { Text(stringResource(R.string.action_allow)) }
        }
    }
    if (showRestricted) {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SettingsOutlinedButton(onClick = onOpenAppInfo) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

/** Rendering only: notification access checks and Android intents stay in NotificationAccess. */
@Composable
internal fun NotificationAccessContent(
    granted: Boolean,
    showRestricted: Boolean,
    onOpenSettings: () -> Unit,
    onChooseFilter: () -> Unit,
    onOpenAppInfo: () -> Unit,
) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SettingsControlRow(
            title = stringResource(R.string.notifications_title),
            supportingText = stringResource(if (granted) R.string.notifications_on else R.string.notifications_off),
        ) {
            if (granted) {
                SettingsOutlinedButton(onClick = onOpenSettings) { Text(stringResource(R.string.action_manage)) }
            } else {
                SettingsButton(onClick = onOpenSettings) { Text(stringResource(R.string.action_allow)) }
            }
        }
        SettingsFilledTonalButton(onClick = onChooseFilter) {
            Text(stringResource(R.string.waypair_extras_filters_action))
        }
        if (showRestricted) {
            Text(
                stringResource(R.string.notifications_restricted),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SettingsOutlinedButton(onClick = onOpenAppInfo) { Text(stringResource(R.string.action_app_info)) }
        }
    }
}

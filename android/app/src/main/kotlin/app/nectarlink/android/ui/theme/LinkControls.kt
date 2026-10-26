// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Surface
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.launch
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonElevation
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SheetState
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.RowScope

/** Three native, content-sized actions. Material retains focus, disabled and click semantics. */
@Composable
fun LinkButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = RoundedCornerShape(50),
    colors: ButtonColors = ButtonDefaults.buttonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = null,
    contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.Button(
    onClick, modifier.heightIn(min = 48.dp), enabled, shape, colors, elevation,
    border, contentPadding, interactionSource, content,
)

@Composable
fun LinkFilledTonalButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = RoundedCornerShape(50),
    colors: ButtonColors = ButtonDefaults.filledTonalButtonColors(
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        contentColor = MaterialTheme.colorScheme.onSurface,
    ),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = null,
    contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.FilledTonalButton(
    onClick, modifier.heightIn(min = 48.dp), enabled, shape, colors, elevation,
    border, contentPadding, interactionSource, content,
)

@Composable
fun LinkOutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = RoundedCornerShape(50),
    colors: ButtonColors = ButtonDefaults.outlinedButtonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.OutlinedButton(
    onClick, modifier.heightIn(min = 48.dp), enabled, shape, colors, elevation,
    border, contentPadding, interactionSource, content,
)

@Composable
fun LinkTextButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = RoundedCornerShape(50),
    colors: ButtonColors = ButtonDefaults.textButtonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = null,
    contentPadding: PaddingValues = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = androidx.compose.material3.TextButton(
    onClick, modifier.heightIn(min = 48.dp), enabled, shape, colors, elevation,
    border, contentPadding, interactionSource, content,
)

/**
 * An original bottom-aligned native Dialog. Its window owns focus and blocks the parent;
 * its lifetime includes collapse. Native Back returns through onBackRequest before closing.
 * Dialog windows use the opaque fallback, never a false sample of another window's pixels.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LinkModalSheet(
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    @Suppress("UNUSED_PARAMETER") sheetState: SheetState = rememberModalBottomSheetState(),
    containerColor: Color = glassModalContainerColor(),
    scrimColor: Color = glassModalScrimColor(),
    onBackRequest: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val reduced = LocalReducedMotion.current
    val latestDismiss by rememberUpdatedState(onDismissRequest)
    val latestBack by rememberUpdatedState(onBackRequest)
    val scope = rememberCoroutineScope()
    val progress = remember { Animatable(0f) }
    var closing by remember { mutableStateOf(false) }
    var dragOffset by remember { mutableFloatStateOf(0f) }
    var naturalContentHeight by remember { mutableIntStateOf(0) }
    var inputFocused by remember { mutableStateOf(false) }
    val clearOwnInputFocus = remember { mutableStateOf<(() -> Unit)?>(null) }
    val dismiss: () -> Unit = {
        if (!closing) {
            closing = true
            scope.launch {
                progress.animateTo(0f, tween(if (reduced) 120 else 220, easing = LinkOutCubic))
                latestDismiss()
            }
        }
    }
    LaunchedEffect(Unit) {
        if (reduced) progress.animateTo(1f, tween(120))
        else progress.animateTo(1f, spring(stiffness = Spring.StiffnessMediumLow))
    }
    Dialog(
        onDismissRequest = {
            if (!closing) {
                if (inputFocused) clearOwnInputFocus.value?.invoke()
                else latestBack?.invoke() ?: dismiss()
            }
        },
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false,
            dismissOnClickOutside = false),
    ) {
        val dialogView = LocalView.current
        val density = LocalDensity.current
        val ownFocusManager = LocalFocusManager.current
        val imeVisible = WindowInsets.ime.getBottom(density) > 0
        var hadOwnIme by remember { mutableStateOf(imeVisible) }
        DisposableEffect(ownFocusManager) {
            clearOwnInputFocus.value = { ownFocusManager.clearFocus() }
            onDispose { clearOwnInputFocus.value = null }
        }
        LaunchedEffect(imeVisible) {
            // Android commonly retains field focus after hiding IME. Clear only this
            // Dialog's focus tree, never the Activity or a nested Dialog's manager.
            if (hadOwnIme && !imeVisible) ownFocusManager.clearFocus()
            hadOwnIme = imeVisible
        }
        SideEffect { (dialogView.parent as? DialogWindowProvider)?.window?.setDimAmount(0f) }
        Box(Modifier.fillMaxSize()) {
            Box(
                Modifier.fillMaxSize().background(scrimColor.copy(alpha = scrimColor.alpha * progress.value))
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null,
                        onClick = dismiss),
            )
            BoxWithConstraints(
                Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing),
                contentAlignment = Alignment.BottomCenter,
            ) {
                // Measurement uses one fixed floating width and finite height cap. Neither
                // depends on the animated card size/inset, so expanding cannot reclassify
                // natural content and feed back into the next expansion decision.
                val contentWidth = (maxWidth - 16.dp).coerceAtLeast(0.dp)
                val naturalHeightLimit = (maxHeight - 8.dp).coerceAtLeast(0.dp)
                val naturalHeight = with(density) { naturalContentHeight.toDp() }
                    .coerceAtMost(naturalHeightLimit)
                val fullHeight = imeVisible || inputFocused ||
                    (naturalContentHeight > 0 && naturalHeight >= naturalHeightLimit)
                val geometrySpec: androidx.compose.animation.core.AnimationSpec<androidx.compose.ui.unit.Dp> =
                    if (reduced) tween(0) else spring(stiffness = Spring.StiffnessMediumLow)
                val inset by animateDpAsState(
                    targetValue = if (fullHeight) 0.dp else 8.dp,
                    animationSpec = geometrySpec, label = "sheet_inset",
                )
                val cardHeight by animateDpAsState(
                    targetValue = if (fullHeight) maxHeight else naturalHeight,
                    animationSpec = geometrySpec, label = "sheet_height",
                )
                val corner by animateDpAsState(
                    targetValue = if (fullHeight) 0.dp else 36.dp,
                    animationSpec = geometrySpec, label = "sheet_corner",
                )
                val bottomCorner by animateDpAsState(
                    targetValue = if (fullHeight) 0.dp else 32.dp,
                    animationSpec = geometrySpec, label = "sheet_bottom_corner",
                )
                Surface(
                    modifier = modifier.fillMaxWidth().padding(
                        start = inset.coerceIn(0.dp, 8.dp), end = inset.coerceIn(0.dp, 8.dp),
                        bottom = inset.coerceIn(0.dp, 8.dp),
                    )
                        .height(cardHeight.coerceIn(0.dp, maxHeight))
                        .graphicsLayer {
                            alpha = progress.value
                            translationY = if (reduced) 0f else (1f - progress.value) * size.height + dragOffset
                        },
                    shape = RoundedCornerShape(
                        topStart = corner.coerceAtLeast(0.dp), topEnd = corner.coerceAtLeast(0.dp),
                        bottomStart = bottomCorner.coerceAtLeast(0.dp), bottomEnd = bottomCorner.coerceAtLeast(0.dp),
                    ),
                    color = containerColor.copy(alpha = 1f), tonalElevation = 0.dp,
                ) {
                    CompositionLocalProvider(
                        LocalLinkSheetDismiss provides dismiss,
                        LocalLinkSheetInputFocus provides { focused -> inputFocused = focused },
                    ) {
                        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                            Column(
                                Modifier.width(contentWidth)
                                    .wrapContentHeight(Alignment.Top, unbounded = true)
                                    // Bound again immediately: scrolling descendants always receive
                                    // finite available constraints even during a height animation.
                                    .heightIn(max = naturalHeightLimit)
                                    .onSizeChanged { naturalContentHeight = it.height },
                                horizontalAlignment = Alignment.CenterHorizontally,
                            ) {
                                Box(
                                    Modifier.fillMaxWidth().heightIn(min = 32.dp)
                                        .pointerInput(reduced, onBackRequest) {
                                            detectVerticalDragGestures(
                                                onDragStart = { dragOffset = 0f },
                                                onVerticalDrag = { change, delta ->
                                                    change.consume()
                                                    dragOffset = (dragOffset + delta).coerceAtLeast(0f)
                                                },
                                                onDragCancel = { dragOffset = 0f },
                                                onDragEnd = {
                                                    if (dragOffset >= 72.dp.toPx()) dismiss() else dragOffset = 0f
                                                },
                                            )
                                        },
                                    contentAlignment = Alignment.TopCenter,
                                ) {
                                    Box(
                                        Modifier.padding(top = 4.dp, bottom = 8.dp)
                                            .size(width = if (fullHeight) 44.dp else 36.dp, height = 4.dp)
                                            .background(MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f), RoundedCornerShape(2.dp)),
                                    )
                                }
                                content()
                            }
                        }
                    }
                }
            }
        }
    }
}

/** Explicit close controls join the same collapse lifetime as Back, outside tap and drag. */
val LocalLinkSheetDismiss = staticCompositionLocalOf<() -> Unit> { {} }

/** Report text-input focus only; button/navigation focus does not expand a sheet. */
val LocalLinkSheetInputFocus = staticCompositionLocalOf<(Boolean) -> Unit> { {} }

/** Original bounded ease-out for non-physical navigation and closing. */
val LinkOutCubic = Easing { fraction -> val inverse = 1f - fraction; 1f - inverse * inverse * inverse }

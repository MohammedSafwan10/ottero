package com.nexdark.ottero.core.feedback

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

@Immutable
data class ConfirmRequest(
    val title: String,
    val message: String,
    val confirmLabel: String,
    val destructive: Boolean = true,
    val onConfirm: suspend () -> Unit,
)

enum class FeedbackTone {
    Neutral,
    Error,
}

/**
 * Global, calm feedback & confirmation controller matching Desktop's
 * `ottero_ui::feedback` contract:
 * - Reversible actions run immediately and offer Undo (`undo`).
 * - Permanent/irreversible actions confirm first (`confirmDestructive`).
 * - Errors surface cleanly without raw stack traces or duplicate spam (`error`).
 */
@Stable
class OtteroFeedbackController(
    val snackbarHostState: SnackbarHostState,
    private val scope: CoroutineScope,
) {
    var activeConfirm by mutableStateOf<ConfirmRequest?>(null)
        private set

    var currentTone by mutableStateOf(FeedbackTone.Neutral)
        private set

    private var lastMessage: String? = null
    private var lastShownAtMs: Long = 0L

    fun info(message: String) {
        showSnackbar(message = message, tone = FeedbackTone.Neutral, actionLabel = null, onAction = null)
    }

    fun error(message: String) {
        showSnackbar(message = message, tone = FeedbackTone.Error, actionLabel = null, onAction = null)
    }

    fun undo(message: String, onUndo: suspend () -> Unit) {
        showSnackbar(
            message = message,
            tone = FeedbackTone.Neutral,
            actionLabel = "Undo",
            onAction = onUndo,
        )
    }

    fun confirmDestructive(
        title: String,
        message: String,
        confirmLabel: String = "Delete",
        onConfirm: suspend () -> Unit,
    ) {
        activeConfirm = ConfirmRequest(
            title = title,
            message = message,
            confirmLabel = confirmLabel,
            destructive = true,
            onConfirm = onConfirm,
        )
    }

    fun dismissConfirm() {
        activeConfirm = null
    }

    fun executeConfirm() {
        val req = activeConfirm ?: return
        activeConfirm = null
        scope.launch {
            runSuspendCatching { req.onConfirm() }
                .onFailure { error(it.message ?: "Action failed") }
        }
    }

    private fun showSnackbar(
        message: String,
        tone: FeedbackTone,
        actionLabel: String?,
        onAction: (suspend () -> Unit)?,
    ) {
        val trimmed = message.trim()
        if (trimmed.isEmpty()) return

        val now = System.currentTimeMillis()
        if (trimmed == lastMessage && actionLabel == null && now - lastShownAtMs < 1500L) {
            return
        }
        lastMessage = trimmed
        lastShownAtMs = now

        scope.launch {
            snackbarHostState.currentSnackbarData?.dismiss()
            currentTone = tone
            val result = snackbarHostState.showSnackbar(
                message = trimmed,
                actionLabel = actionLabel,
                duration = SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed && onAction != null) {
                runSuspendCatching { onAction() }
                    .onFailure { error(it.message ?: "Could not undo") }
            }
        }
    }
}

val LocalOtteroFeedback = compositionLocalOf<OtteroFeedbackController> {
    error("LocalOtteroFeedback not provided")
}

@Composable
fun rememberOtteroFeedback(): OtteroFeedbackController {
    val snackbarHostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    return remember(snackbarHostState, scope) {
        OtteroFeedbackController(snackbarHostState, scope)
    }
}

@Composable
fun OtteroSnackbarHost(
    feedback: OtteroFeedbackController,
    modifier: Modifier = Modifier,
) {
    val palette = LocalOtteroPalette.current
    SnackbarHost(
        hostState = feedback.snackbarHostState,
        modifier = modifier.padding(horizontal = 16.dp, vertical = 8.dp),
    ) { data ->
        val borderColor = if (feedback.currentTone == FeedbackTone.Error) {
            palette.danger.copy(alpha = 0.6f)
        } else {
            palette.border
        }
        Surface(
            modifier = Modifier
                .widthIn(max = 420.dp)
                .fillMaxWidth(),
            shape = RoundedCornerShape(12.dp),
            color = palette.card,
            border = BorderStroke(1.dp, borderColor),
            shadowElevation = if (palette.isDark) 4.dp else 3.dp,
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = data.visuals.message,
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (feedback.currentTone == FeedbackTone.Error) palette.danger else palette.ink,
                    modifier = Modifier.weight(1f),
                )
                val label = data.visuals.actionLabel
                if (label != null) {
                    Spacer(Modifier.width(8.dp))
                    TextButton(onClick = { data.performAction() }) {
                        Text(
                            text = label,
                            style = MaterialTheme.typography.labelLarge,
                            color = palette.ink,
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun OtteroConfirmDialogHost(feedback: OtteroFeedbackController) {
    val req = feedback.activeConfirm ?: return
    val palette = LocalOtteroPalette.current

    AlertDialog(
        onDismissRequest = feedback::dismissConfirm,
        containerColor = palette.card,
        titleContentColor = palette.ink,
        textContentColor = palette.muted,
        shape = RoundedCornerShape(16.dp),
        title = {
            Text(
                text = req.title,
                style = MaterialTheme.typography.titleMedium,
            )
        },
        text = {
            Text(
                text = req.message,
                style = MaterialTheme.typography.bodyMedium,
            )
        },
        dismissButton = {
            TextButton(onClick = feedback::dismissConfirm) {
                Text("Cancel", color = palette.muted)
            }
        },
        confirmButton = {
            Button(
                onClick = feedback::executeConfirm,
                colors = ButtonDefaults.buttonColors(
                    containerColor = if (req.destructive) palette.danger else palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(req.confirmLabel)
            }
        },
    )
}

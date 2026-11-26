package com.nexdark.ottero.feature.capture

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.launch

enum class CaptureKind(val label: String, val placeholder: String) {
    Task("Task", "e.g. Call Sam fri !high"),
    Note("Note", "Note title"),
    Habit("Habit", "e.g. Read 20 pages, Water 8x"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QuickCaptureSheet(
    engine: OtteroEngine,
    initialKind: CaptureKind,
    onDismiss: () -> Unit,
    onOpenNote: (String) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var kind by rememberSaveable { mutableStateOf(initialKind) }
    var text by rememberSaveable { mutableStateOf("") }
    var isSubmitting by remember { mutableStateOf(false) }

    fun submit() {
        val trimmed = text.trim()
        if (trimmed.isEmpty() && kind != CaptureKind.Note) return
        if (isSubmitting) return
        isSubmitting = true
        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        scope.launch {
            when (kind) {
                CaptureKind.Task -> {
                    runSuspendCatching { engine.quickAddTask(trimmed, true) }
                        .onSuccess {
                            feedback.info("Task added")
                            onDismiss()
                        }
                        .onFailure {
                            isSubmitting = false
                            feedback.error(it.message ?: "Could not add task")
                        }
                }

                CaptureKind.Note -> {
                    runSuspendCatching { engine.createNote(trimmed, "") }
                        .onSuccess { note ->
                            onDismiss()
                            onOpenNote(note.id)
                        }
                        .onFailure {
                            isSubmitting = false
                            feedback.error(it.message ?: "Could not create note")
                        }
                }

                CaptureKind.Habit -> {
                    runSuspendCatching { engine.quickAddHabit(trimmed) }
                        .onSuccess {
                            feedback.info("Habit added")
                            onDismiss()
                        }
                        .onFailure {
                            isSubmitting = false
                            feedback.error(it.message ?: "Could not add habit")
                        }
                }
            }
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = palette.card,
        contentColor = palette.ink,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .imePadding()
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = "Quick capture",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CaptureKind.entries.forEach { option ->
                    FilterChip(
                        selected = kind == option,
                        onClick = { kind = option },
                        label = { Text(option.label) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.recessed,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text(kind.placeholder, color = palette.muted) },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { submit() }),
                shape = RoundedCornerShape(12.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedContainerColor = palette.canvas,
                    unfocusedContainerColor = palette.canvas,
                    focusedBorderColor = palette.ink,
                    unfocusedBorderColor = palette.border,
                    focusedTextColor = palette.ink,
                    unfocusedTextColor = palette.ink,
                ),
            )
            Spacer(Modifier.height(2.dp))
            Button(
                enabled = !isSubmitting && (text.isNotBlank() || kind == CaptureKind.Note),
                onClick = ::submit,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(
                    text = when (kind) {
                        CaptureKind.Task -> "Add task"
                        CaptureKind.Note -> "Create note"
                        CaptureKind.Habit -> "Add habit"
                    },
                    style = MaterialTheme.typography.labelLarge,
                )
            }
        }
    }
}

package com.nexdark.ottero.feature.tasks

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.TaskItem
import java.time.LocalDate

private val Priorities = listOf(0 to "None", 1 to "Low", 2 to "Med", 3 to "High")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TaskDetailSheet(
    engine: OtteroEngine,
    task: TaskItem,
    onDismiss: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var title by rememberSaveable(task.id) { mutableStateOf(task.title) }
    var notes by rememberSaveable(task.id) { mutableStateOf(task.notes) }
    var priority by rememberSaveable(task.id) { mutableIntStateOf(task.priority.toInt()) }
    var dueDate by rememberSaveable(task.id) { mutableStateOf(task.dueDate) }
    var isSaving by remember { mutableStateOf(false) }

    val today = remember { LocalDate.now() }
    val datePresets = remember(today) {
        listOf(
            today.toString() to "Today",
            today.plusDays(1).toString() to "Tomorrow",
            today.plusDays(7).toString() to "Next week",
            null to "No date",
        )
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
                text = "Edit task",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )

            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Title", color = palette.muted) },
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

            OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                modifier = Modifier.fillMaxWidth(),
                minLines = 2,
                maxLines = 4,
                placeholder = { Text("Notes", color = palette.muted) },
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

            Text(
                text = "Priority",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Priorities.forEach { (value, label) ->
                    FilterChip(
                        selected = priority == value,
                        onClick = { priority = value },
                        label = { Text(label) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.recessed,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }

            Text(
                text = "Due",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
            val hasCustomDue = dueDate != null && datePresets.none { it.first == dueDate }
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (hasCustomDue) {
                    item(key = "due-custom") {
                        FilterChip(
                            selected = true,
                            onClick = { dueDate = null },
                            label = { Text(dueDate.orEmpty()) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = palette.ink,
                                selectedLabelColor = palette.card,
                            ),
                        )
                    }
                }
                items(
                    items = datePresets,
                    key = { it.second },
                ) { (iso, label) ->
                    FilterChip(
                        selected = dueDate == iso,
                        onClick = { dueDate = iso },
                        label = { Text(label) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.recessed,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }

            Spacer(Modifier.height(4.dp))

            Button(
                enabled = !isSaving && title.isNotBlank(),
                onClick = {
                    isSaving = true
                    scope.launch {
                        runSuspendCatching {
                            engine.updateTask(
                                id = task.id,
                                title = title.trim(),
                                notes = notes,
                                priority = priority,
                                dueDate = dueDate,
                            )
                        }.onSuccess {
                            onDismiss()
                        }.onFailure {
                            isSaving = false
                            feedback.error(it.message ?: "Could not save task")
                        }
                    }
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text("Save", style = MaterialTheme.typography.labelLarge)
            }
        }
    }
}

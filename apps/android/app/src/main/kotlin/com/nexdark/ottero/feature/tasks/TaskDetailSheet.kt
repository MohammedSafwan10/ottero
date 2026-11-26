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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.ProjectItem
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
    var projectId by rememberSaveable(task.id) { mutableStateOf(task.projectId) }
    var projects by remember { mutableStateOf<List<ProjectItem>>(emptyList()) }
    var subtasks by remember { mutableStateOf<List<TaskItem>>(emptyList()) }
    var newSubtask by remember { mutableStateOf("") }
    var isSaving by remember { mutableStateOf(false) }

    LaunchedEffect(engine, task.id) {
        engine.changes.onStart { emit(Unit) }.collectLatest {
            projects = runSuspendCatching { engine.listProjects() }.getOrDefault(emptyList())
            subtasks = runSuspendCatching { engine.listSubtasks(task.id) }.getOrDefault(emptyList())
        }
    }

    val today = remember { LocalDate.now() }
    val datePresets = remember(today) {
        listOf(
            today.toString() to "Today",
            today.plusDays(1).toString() to "Tomorrow",
            today.plusDays(7).toString() to "Next week",
            null to "No date",
        )
    }

    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedContainerColor = palette.canvas,
        unfocusedContainerColor = palette.canvas,
        focusedBorderColor = palette.ink,
        unfocusedBorderColor = palette.border,
        focusedTextColor = palette.ink,
        unfocusedTextColor = palette.ink,
    )

    fun submitSubtask() {
        val text = newSubtask.trim()
        if (text.isEmpty()) return
        scope.launch {
            runSuspendCatching { engine.addSubtask(task.id, text) }
                .onSuccess { newSubtask = "" }
                .onFailure { feedback.error(it.message ?: "Could not add subtask") }
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
                .verticalScroll(rememberScrollState())
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
                colors = fieldColors,
            )

            OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                modifier = Modifier.fillMaxWidth(),
                minLines = 2,
                maxLines = 4,
                placeholder = { Text("Notes", color = palette.muted) },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
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

            if (projects.isNotEmpty()) {
                Text(
                    text = "Project",
                    style = MaterialTheme.typography.labelMedium,
                    color = palette.muted,
                )
                LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    item(key = "proj-none") {
                        FilterChip(
                            selected = projectId == null,
                            onClick = { projectId = null },
                            label = { Text("None") },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = palette.ink,
                                selectedLabelColor = palette.card,
                                containerColor = palette.recessed,
                                labelColor = palette.muted,
                            ),
                        )
                    }
                    items(items = projects, key = { it.id }) { proj ->
                        FilterChip(
                            selected = projectId == proj.id,
                            onClick = { projectId = proj.id },
                            label = { Text(proj.name) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = palette.ink,
                                selectedLabelColor = palette.card,
                                containerColor = palette.recessed,
                                labelColor = palette.muted,
                            ),
                        )
                    }
                }
            }

            Text(
                text = "Subtasks",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
            subtasks.forEach { sub ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(
                        onClick = {
                            scope.launch {
                                runSuspendCatching { engine.toggleTaskDone(sub.id) }
                                    .onFailure { feedback.error(it.message ?: "Could not update subtask") }
                            }
                        },
                        modifier = Modifier.size(36.dp),
                    ) {
                        Icon(
                            imageVector = if (sub.completed) Icons.Outlined.CheckCircle else Icons.Outlined.RadioButtonUnchecked,
                            contentDescription = "Toggle subtask",
                            tint = if (sub.completed) palette.success else palette.muted,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                    Spacer(Modifier.width(4.dp))
                    Text(
                        text = sub.title,
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (sub.completed) palette.muted else palette.ink,
                        textDecoration = if (sub.completed) TextDecoration.LineThrough else null,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(
                        onClick = {
                            scope.launch {
                                runSuspendCatching { engine.deleteTaskPermanently(sub.id) }
                                    .onFailure { feedback.error(it.message ?: "Could not delete subtask") }
                            }
                        },
                        modifier = Modifier.size(36.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.Close,
                            contentDescription = "Remove subtask",
                            tint = palette.muted,
                            modifier = Modifier.size(16.dp),
                        )
                    }
                }
            }

            OutlinedTextField(
                value = newSubtask,
                onValueChange = { newSubtask = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Add a subtask", color = palette.muted) },
                leadingIcon = {
                    Icon(
                        imageVector = Icons.Outlined.Add,
                        contentDescription = null,
                        tint = palette.muted,
                    )
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { submitSubtask() }),
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

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
                            if (projectId != task.projectId) {
                                engine.setTaskProject(task.id, projectId)
                            }
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

package com.nexdark.ottero.feature.tasks

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.TaskAlt
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.MobileTaskFilter
import uniffi.ottero_mobile_ffi.TaskItem

private val TaskFilters = listOf(
    MobileTaskFilter.TODAY to "Today",
    MobileTaskFilter.UPCOMING to "Upcoming",
    MobileTaskFilter.ALL to "All",
    MobileTaskFilter.DONE to "Done",
)

private fun priorityLabel(priority: UByte): String? = when (priority.toInt()) {
    1 -> "Low"
    2 -> "Medium"
    3 -> "High"
    else -> null
}

@Composable
fun TasksScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()
    var filter by rememberSaveable { mutableStateOf(MobileTaskFilter.TODAY) }
    var draft by rememberSaveable { mutableStateOf("") }
    var tasks by remember { mutableStateOf<List<TaskItem>>(emptyList()) }
    var isLoading by remember { mutableStateOf(true) }

    fun submitQuickAdd() {
        val text = draft.trim()
        if (text.isEmpty()) return
        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        scope.launch {
            runSuspendCatching { engine.quickAddTask(text, filter == MobileTaskFilter.TODAY) }
                .onSuccess { draft = "" }
                .onFailure { feedback.error(it.message ?: "Could not add task") }
        }
    }

    LaunchedEffect(engine) {
        combine(
            snapshotFlow { filter },
            engine.changes.onStart { emit(Unit) },
        ) { currentFilter, _ -> currentFilter }.collectLatest { currentFilter ->
            runSuspendCatching { engine.listTasks(currentFilter) }
                .onSuccess {
                    tasks = it
                    isLoading = false
                }
                .onFailure {
                    isLoading = false
                    feedback.error(it.message ?: "Could not load tasks")
                }
        }
    }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .imePadding(),
        contentPadding = PaddingValues(
            start = contentPadding.calculateStartPadding(layoutDirection) + 16.dp,
            end = contentPadding.calculateEndPadding(layoutDirection) + 16.dp,
            top = contentPadding.calculateTopPadding() + 8.dp,
            bottom = contentPadding.calculateBottomPadding() + 88.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item(key = "tasks-filters", contentType = "filters") {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(
                    items = TaskFilters,
                    key = { it.first.name },
                    contentType = { "filter_chip" },
                ) { (value, label) ->
                    val selected = filter == value
                    FilterChip(
                        selected = selected,
                        onClick = { filter = value },
                        label = { Text(label) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.card,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }
        }

        item(key = "tasks-quick-add", contentType = "quick_add") {
            OutlinedTextField(
                value = draft,
                onValueChange = { draft = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Add a task", color = palette.muted) },
                leadingIcon = {
                    Icon(
                        imageVector = Icons.Outlined.Add,
                        contentDescription = null,
                        tint = palette.muted,
                    )
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { submitQuickAdd() }),
                shape = RoundedCornerShape(12.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedContainerColor = palette.card,
                    unfocusedContainerColor = palette.card,
                    focusedBorderColor = palette.ink,
                    unfocusedBorderColor = palette.border,
                    focusedTextColor = palette.ink,
                    unfocusedTextColor = palette.ink,
                ),
            )
        }

        if (!isLoading && tasks.isEmpty()) {
            item(key = "tasks-empty", contentType = "empty") {
                OtteroCard {
                    EmptyPlaceholder(
                        icon = Icons.Outlined.TaskAlt,
                        title = "No tasks here",
                        hint = "Add a task above.",
                    )
                }
            }
        } else {
            items(
                items = tasks,
                key = { "task-${it.id}" },
                contentType = { "task_card" },
            ) { task ->
                OtteroCard {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(
                                onClickLabel = if (task.completed) "Mark incomplete" else "Complete task",
                            ) {
                                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                scope.launch {
                                    runSuspendCatching { engine.toggleTaskDone(task.id) }
                                        .onFailure { feedback.error(it.message ?: "Could not update task") }
                                }
                            },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            imageVector = if (task.completed) Icons.Outlined.CheckCircle else Icons.Outlined.RadioButtonUnchecked,
                            contentDescription = null,
                            tint = if (task.completed) palette.success else palette.muted,
                            modifier = Modifier.size(20.dp),
                        )
                        Spacer(Modifier.width(12.dp))
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = task.title,
                                style = MaterialTheme.typography.bodyLarge,
                                color = if (task.completed) palette.muted else palette.ink,
                                textDecoration = if (task.completed) TextDecoration.LineThrough else null,
                                maxLines = 2,
                                overflow = TextOverflow.Ellipsis,
                            )
                            val meta = buildList {
                                task.dueDate?.let { add(if (task.overdue) "Overdue · $it" else it) }
                                priorityLabel(task.priority)?.let { add(it) }
                            }.joinToString(" · ")
                            if (meta.isNotEmpty()) {
                                Text(
                                    text = meta,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = if (task.overdue || task.priority.toInt() == 3) {
                                        palette.danger
                                    } else {
                                        palette.muted
                                    },
                                )
                            }
                        }
                        IconButton(
                            onClick = {
                                scope.launch {
                                    runSuspendCatching { engine.trashTask(task.id) }
                                        .onSuccess {
                                            feedback.undo("Task moved to trash") {
                                                engine.restoreTask(task.id)
                                            }
                                        }
                                        .onFailure { feedback.error(it.message ?: "Could not trash task") }
                                }
                            },
                            modifier = Modifier.size(48.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Outlined.DeleteOutline,
                                contentDescription = "Move task to trash",
                                tint = palette.muted,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

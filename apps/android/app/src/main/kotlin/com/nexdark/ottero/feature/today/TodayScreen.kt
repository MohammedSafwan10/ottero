package com.nexdark.ottero.feature.today

import androidx.compose.foundation.BorderStroke
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Spa
import androidx.compose.material.icons.outlined.TaskAlt
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.components.SectionHeader
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.HabitItem
import uniffi.ottero_mobile_ffi.NoteListItem
import uniffi.ottero_mobile_ffi.TaskItem
import uniffi.ottero_mobile_ffi.TodaySnapshot
import java.time.LocalDate
import java.time.LocalTime
import java.time.format.DateTimeFormatter

@Composable
fun TodayScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
    onOpenTasks: () -> Unit,
    onOpenNotes: () -> Unit,
    onOpenHabits: () -> Unit,
    onOpenNote: (String) -> Unit,
    onCreateNote: () -> Unit,
    onAddTask: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()
    var snapshot by remember { mutableStateOf<TodaySnapshot?>(null) }

    LaunchedEffect(engine) {
        engine.changes.onStart { emit(Unit) }.collectLatest {
            runSuspendCatching { engine.todaySnapshot() }
                .onSuccess { snapshot = it }
                .onFailure { feedback.error(it.message ?: "Could not load today") }
        }
    }

    val dateKey = snapshot?.dateIso
    val dateLabel = remember(dateKey) {
        val parsed = dateKey?.let { runCatching { LocalDate.parse(it) }.getOrNull() } ?: LocalDate.now()
        parsed.format(DateTimeFormatter.ofPattern("EEEE, MMMM d"))
    }
    val greeting = remember(dateKey) {
        when (LocalTime.now().hour) {
            in 5..11 -> "Good morning"
            in 12..16 -> "Good afternoon"
            else -> "Good evening"
        }
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(
            start = contentPadding.calculateStartPadding(layoutDirection) + 16.dp,
            end = contentPadding.calculateEndPadding(layoutDirection) + 16.dp,
            top = contentPadding.calculateTopPadding() + 8.dp,
            bottom = contentPadding.calculateBottomPadding() + 88.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item(key = "header", contentType = "header") {
            Column(
                modifier = Modifier.padding(bottom = 4.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(
                    text = dateLabel,
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.muted,
                )
                Text(
                    text = greeting,
                    style = MaterialTheme.typography.headlineMedium,
                    color = palette.ink,
                )
            }
        }

        val snap = snapshot
        if (snap != null) {
            item(key = "tasks-header", contentType = "section_header") {
                SectionHeader(
                    title = "Due today",
                    actionLabel = "All tasks",
                    onAction = onOpenTasks,
                )
            }

            if (snap.dueTasks.isEmpty()) {
                item(key = "tasks-empty", contentType = "empty") {
                    OtteroCard {
                        EmptyPlaceholder(
                            icon = Icons.Outlined.TaskAlt,
                            title = "No tasks due today",
                            hint = "Add one to get started.",
                            actionLabel = "Add a task",
                            onAction = onAddTask,
                        )
                    }
                }
            } else {
                items(
                    items = snap.dueTasks,
                    key = { "today-task-${it.id}" },
                    contentType = { "task_row" },
                ) { task ->
                    TodayTaskRow(
                        task = task,
                        onToggle = {
                            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            scope.launch {
                                runSuspendCatching { engine.toggleTaskDone(task.id) }
                                    .onFailure { feedback.error(it.message ?: "Could not update task") }
                            }
                        },
                    )
                }
            }

            if (snap.habits.isNotEmpty()) {
                item(key = "habits-header", contentType = "section_header") {
                    Spacer(Modifier.height(4.dp))
                    SectionHeader(
                        title = "Habits",
                        actionLabel = "All habits",
                        onAction = onOpenHabits,
                    )
                }

                items(
                    items = snap.habits,
                    key = { "today-habit-${it.id}" },
                    contentType = { "habit_row" },
                ) { habit ->
                    TodayHabitRow(
                        habit = habit,
                        onTap = {
                            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            scope.launch {
                                runSuspendCatching { engine.tapHabitToday(habit.id) }
                                    .onFailure { feedback.error(it.message ?: "Could not log habit") }
                            }
                        },
                    )
                }
            }

            item(key = "notes-header", contentType = "section_header") {
                Spacer(Modifier.height(4.dp))
                SectionHeader(
                    title = "Recent notes",
                    actionLabel = "All notes",
                    onAction = onOpenNotes,
                )
            }

            if (snap.recentNotes.isEmpty()) {
                item(key = "notes-empty", contentType = "empty") {
                    OtteroCard {
                        EmptyPlaceholder(
                            icon = Icons.Outlined.Description,
                            title = "No notes yet",
                            hint = "Write your first note.",
                            actionLabel = "New note",
                            onAction = onCreateNote,
                        )
                    }
                }
            } else {
                items(
                    items = snap.recentNotes,
                    key = { "today-note-${it.id}" },
                    contentType = { "note_row" },
                ) { note ->
                    TodayNoteCard(note = note, onClick = { onOpenNote(note.id) })
                }
            }
        }
    }
}

@Composable
private fun TodayTaskRow(
    task: TaskItem,
    onToggle: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    OtteroCard(
        modifier = Modifier.clickable(
            onClickLabel = if (task.completed) "Mark incomplete" else "Complete task",
            onClick = onToggle,
        ),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 32.dp),
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
                    color = palette.ink,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                if (task.overdue) {
                    Text(
                        text = "Overdue",
                        style = MaterialTheme.typography.bodySmall,
                        color = palette.danger,
                    )
                }
            }
        }
    }
}

@Composable
private fun TodayHabitRow(
    habit: HabitItem,
    onTap: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onTap),
        shape = RoundedCornerShape(12.dp),
        color = if (habit.doneToday) palette.success.copy(alpha = 0.14f) else palette.card,
        border = BorderStroke(
            1.dp,
            if (habit.doneToday) palette.success else palette.border,
        ),
    ) {
        Row(
            modifier = Modifier
                .heightIn(min = 48.dp)
                .padding(horizontal = 14.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.weight(1f),
            ) {
                Icon(
                    imageVector = if (habit.doneToday) Icons.Outlined.CheckCircle else Icons.Outlined.Spa,
                    contentDescription = null,
                    tint = if (habit.doneToday) palette.success else palette.muted,
                    modifier = Modifier.size(18.dp),
                )
                Spacer(Modifier.width(10.dp))
                Text(
                    text = habit.name,
                    style = MaterialTheme.typography.bodyMedium,
                    color = palette.ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = "${habit.todayCount}/${habit.targetPerDay}",
                style = MaterialTheme.typography.labelMedium,
                color = if (habit.doneToday) palette.success else palette.muted,
            )
        }
    }
}

@Composable
private fun TodayNoteCard(
    note: NoteListItem,
    onClick: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    OtteroCard(modifier = Modifier.clickable(onClick = onClick)) {
        Text(
            text = note.title,
            style = MaterialTheme.typography.titleSmall,
            color = palette.ink,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (note.preview.isNotBlank()) {
            Spacer(Modifier.height(4.dp))
            Text(
                text = note.preview,
                style = MaterialTheme.typography.bodySmall,
                color = palette.muted,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

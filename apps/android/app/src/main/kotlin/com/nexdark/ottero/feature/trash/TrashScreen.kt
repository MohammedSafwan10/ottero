package com.nexdark.ottero.feature.trash

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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteForever
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.RestoreFromTrash
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.components.OtteroPullToRefresh
import com.nexdark.ottero.core.components.SectionHeader
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.UnifiedTrashSnapshot

@Composable
fun TrashScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()

    var trash by remember { mutableStateOf<UnifiedTrashSnapshot?>(null) }

    LaunchedEffect(engine) {
        engine.changes.onStart { emit(Unit) }.collectLatest {
            runSuspendCatching { engine.unifiedTrashSnapshot() }
                .onSuccess { trash = it }
                .onFailure { feedback.error(it.message ?: "Could not load trash") }
        }
    }

    val snap = trash
    val totalCount = (snap?.notes?.size ?: 0) + (snap?.tasks?.size ?: 0) + (snap?.events?.size ?: 0)

    OtteroPullToRefresh(engine = engine, modifier = Modifier.fillMaxSize()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = contentPadding.calculateStartPadding(layoutDirection) + 16.dp,
                end = contentPadding.calculateEndPadding(layoutDirection) + 16.dp,
                top = contentPadding.calculateTopPadding() + 8.dp,
                bottom = contentPadding.calculateBottomPadding() + 32.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (snap != null && totalCount == 0) {
                item(key = "trash-empty", contentType = "empty") {
                    OtteroCard {
                        EmptyPlaceholder(
                            icon = Icons.Outlined.DeleteOutline,
                            title = "Trash is empty",
                            hint = "Deleted notes, tasks, and events appear here.",
                        )
                    }
                }
            } else if (snap != null) {
                item(key = "trash-actions", contentType = "actions") {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = "$totalCount items in trash",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.muted,
                        )
                        OutlinedButton(
                            onClick = {
                                feedback.confirmDestructive(
                                    title = "Empty trash?",
                                    message = "All $totalCount items will be permanently deleted across synced devices.",
                                    confirmLabel = "Empty trash",
                                ) {
                                    engine.emptyAllTrash()
                                    feedback.info("Trash emptied")
                                }
                            },
                            shape = RoundedCornerShape(10.dp),
                        ) {
                            Text("Empty trash", color = palette.danger)
                        }
                    }
                }

                if (snap.notes.isNotEmpty()) {
                    item(key = "trash-notes-header", contentType = "header") {
                        SectionHeader(title = "Notes")
                    }
                    items(items = snap.notes, key = { "trash-note-${it.id}" }) { note ->
                        TrashRowCard(
                            title = note.title,
                            subtitle = note.preview.takeIf { it.isNotBlank() },
                            onRestore = {
                                scope.launch {
                                    runSuspendCatching { engine.restoreNote(note.id) }
                                        .onSuccess { feedback.info("Note restored") }
                                        .onFailure { feedback.error(it.message ?: "Could not restore note") }
                                }
                            },
                            onDeleteForever = {
                                feedback.confirmDestructive(
                                    title = "Delete permanently?",
                                    message = "\"${note.title}\" cannot be recovered.",
                                    confirmLabel = "Delete",
                                ) {
                                    engine.deleteNotePermanently(note.id)
                                    feedback.info("Deleted permanently")
                                }
                            },
                        )
                    }
                }

                if (snap.tasks.isNotEmpty()) {
                    item(key = "trash-tasks-header", contentType = "header") {
                        SectionHeader(title = "Tasks")
                    }
                    items(items = snap.tasks, key = { "trash-task-${it.id}" }) { task ->
                        TrashRowCard(
                            title = task.title,
                            subtitle = task.dueDate,
                            onRestore = {
                                scope.launch {
                                    runSuspendCatching { engine.restoreTask(task.id) }
                                        .onSuccess { feedback.info("Task restored") }
                                        .onFailure { feedback.error(it.message ?: "Could not restore task") }
                                }
                            },
                            onDeleteForever = {
                                feedback.confirmDestructive(
                                    title = "Delete permanently?",
                                    message = "\"${task.title}\" cannot be recovered.",
                                    confirmLabel = "Delete",
                                ) {
                                    engine.deleteTaskPermanently(task.id)
                                    feedback.info("Deleted permanently")
                                }
                            },
                        )
                    }
                }

                if (snap.events.isNotEmpty()) {
                    item(key = "trash-events-header", contentType = "header") {
                        SectionHeader(title = "Calendar events")
                    }
                    items(items = snap.events, key = { "trash-event-${it.id}" }) { ev ->
                        TrashRowCard(
                            title = ev.title,
                            subtitle = ev.dateIso,
                            onRestore = {
                                scope.launch {
                                    runSuspendCatching { engine.restoreEvent(ev.id) }
                                        .onSuccess { feedback.info("Event restored") }
                                        .onFailure { feedback.error(it.message ?: "Could not restore event") }
                                }
                            },
                            onDeleteForever = {
                                feedback.confirmDestructive(
                                    title = "Delete permanently?",
                                    message = "\"${ev.title}\" cannot be recovered.",
                                    confirmLabel = "Delete",
                                ) {
                                    engine.deleteEventPermanently(ev.id)
                                    feedback.info("Deleted permanently")
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun TrashRowCard(
    title: String,
    subtitle: String?,
    onRestore: () -> Unit,
    onDeleteForever: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    OtteroCard {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleSmall,
                    color = palette.ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (!subtitle.isNullOrBlank()) {
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = palette.muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            IconButton(
                onClick = onRestore,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    imageVector = Icons.Outlined.RestoreFromTrash,
                    contentDescription = "Restore",
                    tint = palette.ink,
                )
            }
            IconButton(
                onClick = onDeleteForever,
                modifier = Modifier.size(48.dp),
            ) {
                Icon(
                    imageVector = Icons.Outlined.DeleteForever,
                    contentDescription = "Delete permanently",
                    tint = palette.danger,
                )
            }
        }
    }
}

package com.nexdark.ottero.feature.notes

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.NoteListItem

@OptIn(FlowPreview::class)
@Composable
fun NotesListScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
    onOpenNote: (String) -> Unit,
    onShowMessage: (String) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val layoutDirection = LocalLayoutDirection.current
    var query by rememberSaveable { mutableStateOf("") }
    var notes by remember { mutableStateOf<List<NoteListItem>>(emptyList()) }
    var isLoading by remember { mutableStateOf(true) }

    LaunchedEffect(engine) {
        combine(
            snapshotFlow { query }.debounce(150),
            engine.changes.onStart { emit(Unit) },
        ) { q, _ -> q }.collectLatest { q ->
            runCatching { engine.listNotes(q.ifBlank { null }) }
                .onSuccess {
                    notes = it
                    isLoading = false
                }
                .onFailure {
                    isLoading = false
                    onShowMessage(it.message ?: "Could not load notes")
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
            top = contentPadding.calculateTopPadding() + 16.dp,
            bottom = contentPadding.calculateBottomPadding() + 88.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item(key = "notes-title", contentType = "header") {
            Text(
                text = "Notes",
                style = MaterialTheme.typography.headlineMedium,
                color = palette.ink,
            )
        }

        item(key = "notes-search", contentType = "search") {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Search notes", color = palette.muted) },
                leadingIcon = {
                    Icon(
                        imageVector = Icons.Outlined.Search,
                        contentDescription = null,
                        tint = palette.muted,
                    )
                },
                trailingIcon = if (query.isNotEmpty()) {
                    {
                        IconButton(onClick = { query = "" }) {
                            Icon(
                                imageVector = Icons.Outlined.Close,
                                contentDescription = "Clear search",
                                tint = palette.muted,
                            )
                        }
                    }
                } else {
                    null
                },
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

        if (!isLoading && notes.isEmpty()) {
            item(key = "notes-empty", contentType = "empty") {
                OtteroCard {
                    EmptyPlaceholder(
                        icon = Icons.Outlined.Description,
                        title = if (query.isBlank()) "No notes yet" else "No matches",
                        hint = if (query.isBlank()) "Tap + to write your first note." else "Try fewer or different words.",
                    )
                }
            }
        } else {
            items(
                items = notes,
                key = { "note-${it.id}" },
                contentType = { "note_card" },
            ) { note ->
                OtteroCard(modifier = Modifier.clickable { onOpenNote(note.id) }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (note.pinned) {
                            Icon(
                                imageVector = Icons.Outlined.PushPin,
                                contentDescription = "Pinned",
                                tint = palette.muted,
                                modifier = Modifier.size(14.dp),
                            )
                            Spacer(Modifier.width(6.dp))
                        }
                        Text(
                            text = note.title,
                            style = MaterialTheme.typography.titleSmall,
                            color = palette.ink,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
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
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NoteEditorScreen(
    engine: OtteroEngine,
    noteId: String,
    isNewNote: Boolean,
    onShowMessage: (String) -> Unit,
    onTrashed: (String) -> Unit,
    onBack: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val scope = rememberCoroutineScope()

    var title by rememberSaveable(noteId) { mutableStateOf("") }
    var body by rememberSaveable(noteId) { mutableStateOf("") }
    var pinned by rememberSaveable(noteId) { mutableStateOf(false) }
    var loaded by rememberSaveable(noteId) { mutableStateOf(false) }
    var lastSavedTitle by rememberSaveable(noteId) { mutableStateOf("") }
    var lastSavedBody by rememberSaveable(noteId) { mutableStateOf("") }
    var isClosing by remember { mutableStateOf(false) }

    val isDirty = loaded && (title != lastSavedTitle || body != lastSavedBody)

    LaunchedEffect(noteId) {
        if (loaded) return@LaunchedEffect
        runCatching { engine.getNote(noteId) }
            .onSuccess { detail ->
                val initialTitle = if (detail.title == "Untitled") "" else detail.title
                title = initialTitle
                body = detail.body
                pinned = detail.pinned
                lastSavedTitle = initialTitle
                lastSavedBody = detail.body
                loaded = true
            }
            .onFailure {
                onShowMessage(it.message ?: "Could not open note")
                onBack()
            }
    }

    // Debounced 500 ms autosave matching Desktop NotesView; skipped when clean.
    LaunchedEffect(loaded, title, body) {
        if (!loaded || (title == lastSavedTitle && body == lastSavedBody)) return@LaunchedEffect
        delay(500)
        runCatching { engine.updateNote(noteId, title, body) }
            .onSuccess {
                lastSavedTitle = title
                lastSavedBody = body
            }
            .onFailure { onShowMessage(it.message ?: "Could not save note") }
    }

    // Flush dirty edits on app background/stop via application-scoped dispatcher.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) {
        if (isDirty) {
            engine.flushNoteAsync(noteId, title, body)
            lastSavedTitle = title
            lastSavedBody = body
        }
    }

    fun handleBack() {
        if (isClosing) return
        isClosing = true
        scope.launch {
            if (loaded && isNewNote && title.isBlank() && body.isBlank()) {
                runCatching { engine.trashNote(noteId) }
            } else if (isDirty) {
                runCatching { engine.updateNote(noteId, title, body) }
                    .onSuccess {
                        lastSavedTitle = title
                        lastSavedBody = body
                    }
                    .onFailure { onShowMessage(it.message ?: "Could not save note") }
            }
            onBack()
        }
    }

    BackHandler(enabled = true) {
        handleBack()
    }

    Scaffold(
        containerColor = palette.canvas,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = palette.canvas,
                    navigationIconContentColor = palette.ink,
                    actionIconContentColor = palette.muted,
                ),
                title = {},
                navigationIcon = {
                    IconButton(onClick = ::handleBack, enabled = !isClosing) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                actions = {
                    IconButton(
                        enabled = loaded && !isClosing,
                        onClick = {
                            val previous = pinned
                            val next = !previous
                            pinned = next
                            scope.launch {
                                runCatching { engine.setNotePinned(noteId, next) }
                                    .onFailure {
                                        pinned = previous
                                        onShowMessage(it.message ?: "Could not update pin")
                                    }
                            }
                        },
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.PushPin,
                            contentDescription = if (pinned) "Unpin note" else "Pin note",
                            tint = if (pinned) palette.ink else palette.muted,
                        )
                    }
                    IconButton(
                        enabled = loaded && !isClosing,
                        onClick = {
                            if (isClosing) return@IconButton
                            isClosing = true
                            scope.launch {
                                runCatching { engine.trashNote(noteId) }
                                    .onSuccess { onTrashed(noteId) }
                                    .onFailure {
                                        isClosing = false
                                        onShowMessage(it.message ?: "Could not trash note")
                                    }
                            }
                        },
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.DeleteOutline,
                            contentDescription = "Move to trash",
                        )
                    }
                },
            )
        },
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
                .consumeWindowInsets(innerPadding)
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (loaded) {
                BasicTextField(
                    value = title,
                    onValueChange = { title = it },
                    modifier = Modifier.fillMaxWidth(),
                    textStyle = MaterialTheme.typography.headlineMedium.copy(color = palette.ink),
                    cursorBrush = SolidColor(palette.ink),
                    decorationBox = { inner ->
                        if (title.isEmpty()) {
                            Text(
                                text = "Untitled",
                                style = MaterialTheme.typography.headlineMedium,
                                color = palette.muted.copy(alpha = 0.6f),
                            )
                        }
                        inner()
                    },
                )
                BasicTextField(
                    value = body,
                    onValueChange = { body = it },
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 480.dp),
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = palette.ink),
                    cursorBrush = SolidColor(palette.ink),
                    decorationBox = { inner ->
                        if (body.isEmpty()) {
                            Text(
                                text = "Start writing…",
                                style = MaterialTheme.typography.bodyLarge,
                                color = palette.muted.copy(alpha = 0.6f),
                            )
                        }
                        inner()
                    },
                )
            }
        }
    }
}

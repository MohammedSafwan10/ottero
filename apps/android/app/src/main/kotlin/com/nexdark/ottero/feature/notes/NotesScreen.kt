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
import androidx.compose.foundation.lazy.LazyRow
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
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
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
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroSnackbarHost
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.NoteListItem
import uniffi.ottero_mobile_ffi.TagCount

@OptIn(FlowPreview::class)
@Composable
fun NotesListScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
    onOpenNote: (String) -> Unit,
    onCreateNote: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    var query by rememberSaveable { mutableStateOf("") }
    var selectedTag by rememberSaveable { mutableStateOf<String?>(null) }
    var tags by remember { mutableStateOf<List<TagCount>>(emptyList()) }
    var notes by remember { mutableStateOf<List<NoteListItem>>(emptyList()) }
    var isLoading by remember { mutableStateOf(true) }

    LaunchedEffect(engine) {
        combine(
            snapshotFlow { query }.debounce(150),
            snapshotFlow { selectedTag },
            engine.changes.onStart { emit(Unit) },
        ) { q, tag, _ -> q to tag }.collectLatest { (q, tag) ->
            runSuspendCatching {
                val loadedTags = engine.listTags()
                val loadedNotes = when {
                    q.isNotBlank() -> engine.listNotes(q)
                    tag != null -> engine.listNotesByTag(tag)
                    else -> engine.listNotes(null)
                }
                loadedTags to loadedNotes
            }.onSuccess { (loadedTags, loadedNotes) ->
                tags = loadedTags
                if (selectedTag != null && loadedTags.none { it.name == selectedTag }) {
                    selectedTag = null
                }
                notes = loadedNotes
                isLoading = false
            }.onFailure {
                isLoading = false
                feedback.error(it.message ?: "Could not load notes")
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

        if (tags.isNotEmpty() && query.isBlank()) {
            item(key = "notes-tags", contentType = "tags") {
                LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(
                        items = tags,
                        key = { "tag-${it.name}" },
                        contentType = { "tag_chip" },
                    ) { tag ->
                        val active = selectedTag == tag.name
                        FilterChip(
                            selected = active,
                            onClick = { selectedTag = if (active) null else tag.name },
                            label = { Text("#${tag.name}") },
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
        }

        if (!isLoading && notes.isEmpty()) {
            item(key = "notes-empty", contentType = "empty") {
                OtteroCard {
                    EmptyPlaceholder(
                        icon = Icons.Outlined.Description,
                        title = if (query.isBlank() && selectedTag == null) "No notes yet" else "No matches",
                        hint = if (query.isBlank() && selectedTag == null) {
                            "Write your first note."
                        } else {
                            "Try another search or tag."
                        },
                        actionLabel = if (query.isBlank() && selectedTag == null) "New note" else null,
                        onAction = if (query.isBlank() && selectedTag == null) onCreateNote else null,
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
    onTrashed: (String) -> Unit,
    onBack: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val scope = rememberCoroutineScope()

    var title by rememberSaveable(noteId) { mutableStateOf("") }
    var bodyText by rememberSaveable(noteId) { mutableStateOf("") }
    var bodyValue by remember(noteId) {
        mutableStateOf(TextFieldValue(bodyText, TextRange(bodyText.length)))
    }
    var pinned by rememberSaveable(noteId) { mutableStateOf(false) }
    var loaded by rememberSaveable(noteId) { mutableStateOf(false) }
    var lastSavedTitle by rememberSaveable(noteId) { mutableStateOf("") }
    var lastSavedBody by rememberSaveable(noteId) { mutableStateOf("") }
    var isClosing by remember { mutableStateOf(false) }

    val isDirty = loaded && (title != lastSavedTitle || bodyText != lastSavedBody)

    LaunchedEffect(noteId) {
        if (loaded) {
            bodyValue = TextFieldValue(bodyText, TextRange(bodyText.length))
            return@LaunchedEffect
        }
        runSuspendCatching { engine.getNote(noteId) }
            .onSuccess { detail ->
                val initialTitle = if (detail.title == "Untitled") "" else detail.title
                title = initialTitle
                bodyText = detail.body
                bodyValue = TextFieldValue(detail.body, TextRange(detail.body.length))
                pinned = detail.pinned
                lastSavedTitle = initialTitle
                lastSavedBody = detail.body
                loaded = true
            }
            .onFailure {
                feedback.error(it.message ?: "Could not open note")
                onBack()
            }
    }

    LaunchedEffect(loaded, title, bodyText) {
        if (!loaded || (title == lastSavedTitle && bodyText == lastSavedBody)) return@LaunchedEffect
        delay(500)
        runSuspendCatching { engine.updateNote(noteId, title, bodyText) }
            .onSuccess {
                lastSavedTitle = title
                lastSavedBody = bodyText
            }
            .onFailure { feedback.error(it.message ?: "Could not save note") }
    }

    LifecycleEventEffect(Lifecycle.Event.ON_STOP) {
        if (isDirty) {
            engine.flushNoteAsync(noteId, title, bodyText)
            lastSavedTitle = title
            lastSavedBody = bodyText
        }
    }

    fun handleBack() {
        if (isClosing) return
        isClosing = true
        scope.launch {
            if (loaded && isNewNote && title.isBlank() && bodyText.isBlank()) {
                runSuspendCatching { engine.trashNote(noteId) }
            } else if (isDirty) {
                runSuspendCatching { engine.updateNote(noteId, title, bodyText) }
                    .onSuccess {
                        lastSavedTitle = title
                        lastSavedBody = bodyText
                    }
                    .onFailure { feedback.error(it.message ?: "Could not save note") }
            }
            onBack()
        }
    }

    BackHandler(enabled = true) {
        handleBack()
    }

    Scaffold(
        containerColor = palette.canvas,
        snackbarHost = { OtteroSnackbarHost(feedback) },
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
                                runSuspendCatching { engine.setNotePinned(noteId, next) }
                                    .onFailure {
                                        pinned = previous
                                        feedback.error(it.message ?: "Could not update pin")
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
                                runSuspendCatching { engine.trashNote(noteId) }
                                    .onSuccess { onTrashed(noteId) }
                                    .onFailure {
                                        isClosing = false
                                        feedback.error(it.message ?: "Could not trash note")
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
                .imePadding(),
        ) {
            Column(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
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
                        value = bodyValue,
                        onValueChange = { next ->
                            bodyValue = next
                            bodyText = next.text
                        },
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 420.dp),
                        textStyle = MaterialTheme.typography.bodyLarge.copy(color = palette.ink),
                        cursorBrush = SolidColor(palette.ink),
                        decorationBox = { inner ->
                            if (bodyValue.text.isEmpty()) {
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

            if (loaded) {
                NoteMarkdownToolbar(
                    value = bodyValue,
                    onValueChange = { next ->
                        bodyValue = next
                        bodyText = next.text
                    },
                )
            }
        }
    }
}

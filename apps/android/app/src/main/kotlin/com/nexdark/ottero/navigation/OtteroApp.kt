package com.nexdark.ottero.navigation

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WbSunny
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.NavigationRailItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.feature.notes.NoteEditorScreen
import com.nexdark.ottero.feature.notes.NotesListScreen
import com.nexdark.ottero.feature.settings.SettingsScreen
import com.nexdark.ottero.feature.tasks.TasksScreen
import com.nexdark.ottero.feature.today.TodayScreen
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import kotlinx.coroutines.launch

private enum class TopDestination(val label: String, val icon: ImageVector) {
    Today("Today", Icons.Outlined.WbSunny),
    Notes("Notes", Icons.Outlined.Description),
    Tasks("Tasks", Icons.Outlined.CheckCircle),
    Settings("Settings", Icons.Outlined.Settings),
}

@Composable
fun OtteroApp(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val scope = rememberCoroutineScope()
    val snackbarHostState = remember { SnackbarHostState() }

    var tab by rememberSaveable { mutableStateOf(TopDestination.Today) }
    var activeNoteId by rememberSaveable { mutableStateOf<String?>(null) }
    var activeNoteIsNew by rememberSaveable { mutableStateOf(false) }
    var isCreatingNote by remember { mutableStateOf(false) }

    fun showMessage(message: String) {
        scope.launch {
            snackbarHostState.currentSnackbarData?.dismiss()
            snackbarHostState.showSnackbar(
                message = message,
                duration = SnackbarDuration.Short,
            )
        }
    }

    fun showUndo(message: String, onUndo: suspend () -> Unit) {
        scope.launch {
            snackbarHostState.currentSnackbarData?.dismiss()
            val result = snackbarHostState.showSnackbar(
                message = message,
                actionLabel = "Undo",
                duration = SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed) {
                runCatching { onUndo() }
                    .onFailure { showMessage("Could not undo") }
            }
        }
    }

    val openNoteId = activeNoteId
    if (openNoteId != null) {
        NoteEditorScreen(
            engine = engine,
            noteId = openNoteId,
            isNewNote = activeNoteIsNew,
            onShowMessage = ::showMessage,
            onTrashed = { trashedId ->
                activeNoteId = null
                activeNoteIsNew = false
                showUndo("Note moved to trash") { engine.restoreNote(trashedId) }
            },
            onBack = {
                activeNoteId = null
                activeNoteIsNew = false
            },
        )
        return
    }

    BackHandler(enabled = tab != TopDestination.Today) {
        tab = TopDestination.Today
    }

    val density = LocalDensity.current
    val windowWidthDp = with(density) { LocalWindowInfo.current.containerSize.width.toDp() }
    val useRail = windowWidthDp >= 600.dp

    val fab: @Composable () -> Unit = {
        if (tab == TopDestination.Notes) {
            FloatingActionButton(
                onClick = {
                    if (isCreatingNote) return@FloatingActionButton
                    isCreatingNote = true
                    scope.launch {
                        runCatching { engine.createNote("", "") }
                            .onSuccess {
                                activeNoteIsNew = true
                                activeNoteId = it.id
                            }
                            .onFailure { showMessage(it.message ?: "Could not create note") }
                        isCreatingNote = false
                    }
                },
                containerColor = palette.ink,
                contentColor = palette.card,
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = "New note",
                )
            }
        }
    }

    if (useRail) {
        Row(modifier = Modifier.fillMaxSize()) {
            NavigationRail(containerColor = palette.recessed) {
                TopDestination.entries.forEach { dest ->
                    NavigationRailItem(
                        selected = tab == dest,
                        onClick = { tab = dest },
                        icon = { Icon(dest.icon, contentDescription = dest.label) },
                        label = { Text(dest.label) },
                        colors = NavigationRailItemDefaults.colors(
                            selectedIconColor = palette.ink,
                            selectedTextColor = palette.ink,
                            unselectedIconColor = palette.muted,
                            unselectedTextColor = palette.muted,
                            indicatorColor = palette.card,
                        ),
                    )
                }
            }
            Scaffold(
                modifier = Modifier.weight(1f),
                containerColor = palette.canvas,
                snackbarHost = { SnackbarHost(snackbarHostState) },
                floatingActionButton = fab,
            ) { innerPadding ->
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .consumeWindowInsets(innerPadding),
                    contentAlignment = Alignment.TopCenter,
                ) {
                    Box(modifier = Modifier.widthIn(max = 720.dp).fillMaxSize()) {
                        TabContent(
                            tab = tab,
                            engine = engine,
                            themePreference = themePreference,
                            onThemeChange = onThemeChange,
                            contentPadding = innerPadding,
                            onSelectTab = { tab = it },
                            onOpenNote = { id ->
                                activeNoteIsNew = false
                                activeNoteId = id
                            },
                            onShowMessage = ::showMessage,
                            onShowUndo = ::showUndo,
                        )
                    }
                }
            }
        }
    } else {
        Scaffold(
            containerColor = palette.canvas,
            snackbarHost = { SnackbarHost(snackbarHostState) },
            floatingActionButton = fab,
            bottomBar = {
                NavigationBar(containerColor = palette.recessed) {
                    TopDestination.entries.forEach { dest ->
                        NavigationBarItem(
                            selected = tab == dest,
                            onClick = { tab = dest },
                            icon = { Icon(dest.icon, contentDescription = dest.label) },
                            label = { Text(dest.label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = palette.ink,
                                selectedTextColor = palette.ink,
                                unselectedIconColor = palette.muted,
                                unselectedTextColor = palette.muted,
                                indicatorColor = palette.card,
                            ),
                        )
                    }
                }
            },
        ) { innerPadding ->
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .consumeWindowInsets(innerPadding),
            ) {
                TabContent(
                    tab = tab,
                    engine = engine,
                    themePreference = themePreference,
                    onThemeChange = onThemeChange,
                    contentPadding = innerPadding,
                    onSelectTab = { tab = it },
                    onOpenNote = { id ->
                        activeNoteIsNew = false
                        activeNoteId = id
                    },
                    onShowMessage = ::showMessage,
                    onShowUndo = ::showUndo,
                )
            }
        }
    }
}

@Composable
private fun TabContent(
    tab: TopDestination,
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    contentPadding: PaddingValues,
    onSelectTab: (TopDestination) -> Unit,
    onOpenNote: (String) -> Unit,
    onShowMessage: (String) -> Unit,
    onShowUndo: (String, suspend () -> Unit) -> Unit,
) {
    when (tab) {
        TopDestination.Today -> TodayScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenTasks = { onSelectTab(TopDestination.Tasks) },
            onOpenNotes = { onSelectTab(TopDestination.Notes) },
            onOpenNote = onOpenNote,
            onShowMessage = onShowMessage,
        )

        TopDestination.Notes -> NotesListScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenNote = onOpenNote,
            onShowMessage = onShowMessage,
        )

        TopDestination.Tasks -> TasksScreen(
            engine = engine,
            contentPadding = contentPadding,
            onShowMessage = onShowMessage,
            onShowUndo = onShowUndo,
        )

        TopDestination.Settings -> SettingsScreen(
            engine = engine,
            themePreference = themePreference,
            onThemeChange = onThemeChange,
            contentPadding = contentPadding,
        )
    }
}

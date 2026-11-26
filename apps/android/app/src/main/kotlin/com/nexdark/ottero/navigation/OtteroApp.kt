package com.nexdark.ottero.navigation

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Spa
import androidx.compose.material.icons.outlined.WbSunny
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.NavigationDrawerItem
import androidx.compose.material3.NavigationDrawerItemDefaults
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.NavigationRailItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.R
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroTopChrome
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroSnackbarHost
import com.nexdark.ottero.feature.capture.CaptureKind
import com.nexdark.ottero.feature.capture.QuickCaptureSheet
import com.nexdark.ottero.feature.habits.HabitsScreen
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
    Habits("Habits", Icons.Outlined.Spa),
}

@Composable
fun OtteroApp(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val scope = rememberCoroutineScope()

    var tab by rememberSaveable { mutableStateOf(TopDestination.Today) }
    var activeNoteId by rememberSaveable { mutableStateOf<String?>(null) }
    var activeNoteIsNew by rememberSaveable { mutableStateOf(false) }
    var showSettings by rememberSaveable { mutableStateOf(false) }
    var captureSheetKind by rememberSaveable { mutableStateOf<CaptureKind?>(null) }
    var isCreatingNote by remember { mutableStateOf(false) }

    fun createAndOpenNote() {
        if (isCreatingNote) return
        isCreatingNote = true
        scope.launch {
            runSuspendCatching { engine.createNote("", "") }
                .onSuccess {
                    activeNoteIsNew = true
                    activeNoteId = it.id
                }
                .onFailure { feedback.error(it.message ?: "Could not create note") }
            isCreatingNote = false
        }
    }

    val openNoteId = activeNoteId
    if (openNoteId != null) {
        NoteEditorScreen(
            engine = engine,
            noteId = openNoteId,
            isNewNote = activeNoteIsNew,
            onTrashed = { trashedId ->
                activeNoteId = null
                activeNoteIsNew = false
                feedback.undo("Note moved to trash") { engine.restoreNote(trashedId) }
            },
            onBack = {
                activeNoteId = null
                activeNoteIsNew = false
            },
        )
        return
    }

    if (showSettings) {
        SettingsScreen(
            engine = engine,
            themePreference = themePreference,
            onThemeChange = onThemeChange,
            onBack = { showSettings = false },
        )
        return
    }

    val density = LocalDensity.current
    val windowWidthDp = with(density) { LocalWindowInfo.current.containerSize.width.toDp() }
    val useRail = windowWidthDp >= 600.dp

    val fab: @Composable () -> Unit = {
        FloatingActionButton(
            onClick = {
                when (tab) {
                    TopDestination.Notes -> createAndOpenNote()
                    TopDestination.Today -> captureSheetKind = CaptureKind.Task
                    TopDestination.Tasks -> captureSheetKind = CaptureKind.Task
                    TopDestination.Habits -> captureSheetKind = CaptureKind.Habit
                }
            },
            containerColor = palette.ink,
            contentColor = palette.card,
        ) {
            Icon(
                imageVector = Icons.Outlined.Add,
                contentDescription = if (tab == TopDestination.Notes) "New note" else "Quick capture",
            )
        }
    }

    val activeCaptureKind = captureSheetKind
    if (activeCaptureKind != null) {
        QuickCaptureSheet(
            engine = engine,
            initialKind = activeCaptureKind,
            onDismiss = { captureSheetKind = null },
            onOpenNote = { id ->
                activeNoteIsNew = false
                activeNoteId = id
            },
        )
    }

    if (useRail) {
        BackHandler(enabled = tab != TopDestination.Today) {
            tab = TopDestination.Today
        }
        Row(modifier = Modifier.fillMaxSize()) {
            NavigationRail(
                containerColor = palette.recessed,
                header = {
                    Image(
                        painter = painterResource(id = R.drawable.ic_launcher_foreground),
                        contentDescription = "Ottero",
                        modifier = Modifier
                            .padding(top = 12.dp, bottom = 8.dp)
                            .size(36.dp)
                            .clip(RoundedCornerShape(10.dp)),
                    )
                },
            ) {
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
                Spacer(Modifier.weight(1f))
                IconButton(
                    onClick = { showSettings = true },
                    modifier = Modifier.padding(bottom = 12.dp),
                ) {
                    Icon(
                        imageVector = Icons.Outlined.Settings,
                        contentDescription = "Settings",
                        tint = palette.muted,
                    )
                }
            }
            Scaffold(
                modifier = Modifier.weight(1f),
                containerColor = palette.canvas,
                topBar = {
                    OtteroTopChrome(
                        title = tab.label,
                        showDrawerButton = false,
                        onOpenDrawer = {},
                    )
                },
                snackbarHost = { OtteroSnackbarHost(feedback) },
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
                            contentPadding = innerPadding,
                            onSelectTab = { tab = it },
                            onOpenNote = { id ->
                                activeNoteIsNew = false
                                activeNoteId = id
                            },
                            onCreateNote = ::createAndOpenNote,
                            onOpenQuickCapture = { captureSheetKind = it },
                        )
                    }
                }
            }
        }
    } else {
        val drawerState = rememberDrawerState(initialValue = DrawerValue.Closed)

        BackHandler(enabled = drawerState.isOpen || tab != TopDestination.Today) {
            if (drawerState.isOpen) {
                scope.launch { drawerState.close() }
            } else {
                tab = TopDestination.Today
            }
        }

        ModalNavigationDrawer(
            drawerState = drawerState,
            drawerContent = {
                ModalDrawerSheet(
                    drawerContainerColor = palette.recessed,
                    drawerContentColor = palette.ink,
                    modifier = Modifier.widthIn(max = 300.dp),
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(horizontal = 12.dp, vertical = 16.dp),
                    ) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(start = 12.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Box(
                                    modifier = Modifier
                                        .size(32.dp)
                                        .clip(RoundedCornerShape(9.dp)),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Image(
                                        painter = painterResource(id = R.drawable.ic_launcher_foreground),
                                        contentDescription = null,
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier
                                            .fillMaxSize()
                                            .graphicsLayer(scaleX = 1.48f, scaleY = 1.48f),
                                    )
                                }
                                Spacer(Modifier.width(12.dp))
                                Text(
                                    text = "Ottero",
                                    style = MaterialTheme.typography.titleMedium,
                                    color = palette.ink,
                                )
                            }
                            IconButton(
                                onClick = {
                                    scope.launch { drawerState.close() }
                                    showSettings = true
                                },
                            ) {
                                Icon(
                                    imageVector = Icons.Outlined.Settings,
                                    contentDescription = "Settings",
                                    tint = palette.muted,
                                )
                            }
                        }

                        Spacer(Modifier.height(12.dp))
                        HorizontalDivider(color = palette.border)
                        Spacer(Modifier.height(12.dp))

                        TopDestination.entries.forEach { dest ->
                            NavigationDrawerItem(
                                label = { Text(dest.label) },
                                icon = { Icon(dest.icon, contentDescription = null) },
                                selected = tab == dest,
                                onClick = {
                                    tab = dest
                                    scope.launch { drawerState.close() }
                                },
                                colors = NavigationDrawerItemDefaults.colors(
                                    selectedContainerColor = palette.card,
                                    selectedIconColor = palette.ink,
                                    selectedTextColor = palette.ink,
                                    unselectedContainerColor = palette.recessed,
                                    unselectedIconColor = palette.muted,
                                    unselectedTextColor = palette.muted,
                                ),
                                shape = RoundedCornerShape(10.dp),
                            )
                        }
                    }
                }
            },
        ) {
            Scaffold(
                containerColor = palette.canvas,
                topBar = {
                    OtteroTopChrome(
                        title = tab.label,
                        showDrawerButton = true,
                        onOpenDrawer = { scope.launch { drawerState.open() } },
                    )
                },
                snackbarHost = { OtteroSnackbarHost(feedback) },
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
                        contentPadding = innerPadding,
                        onSelectTab = { tab = it },
                        onOpenNote = { id ->
                            activeNoteIsNew = false
                            activeNoteId = id
                        },
                        onCreateNote = ::createAndOpenNote,
                        onOpenQuickCapture = { captureSheetKind = it },
                    )
                }
            }
        }
    }
}

@Composable
private fun TabContent(
    tab: TopDestination,
    engine: OtteroEngine,
    contentPadding: PaddingValues,
    onSelectTab: (TopDestination) -> Unit,
    onOpenNote: (String) -> Unit,
    onCreateNote: () -> Unit,
    onOpenQuickCapture: (CaptureKind) -> Unit,
) {
    when (tab) {
        TopDestination.Today -> TodayScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenTasks = { onSelectTab(TopDestination.Tasks) },
            onOpenNotes = { onSelectTab(TopDestination.Notes) },
            onOpenHabits = { onSelectTab(TopDestination.Habits) },
            onOpenNote = onOpenNote,
            onCreateNote = onCreateNote,
            onAddTask = { onOpenQuickCapture(CaptureKind.Task) },
        )

        TopDestination.Notes -> NotesListScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenNote = onOpenNote,
            onCreateNote = onCreateNote,
        )

        TopDestination.Tasks -> TasksScreen(
            engine = engine,
            contentPadding = contentPadding,
        )

        TopDestination.Habits -> HabitsScreen(
            engine = engine,
            contentPadding = contentPadding,
        )
    }
}

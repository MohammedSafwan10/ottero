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
import androidx.compose.material.icons.outlined.CalendarToday
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Spa
import androidx.compose.material.icons.outlined.Timer
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.nexdark.ottero.R
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroTopChrome
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroSnackbarHost
import com.nexdark.ottero.core.prefs.LocalOtteroPrefs
import com.nexdark.ottero.core.prefs.OtteroPrefsState
import com.nexdark.ottero.core.prefs.StartupDestination
import com.nexdark.ottero.feature.calendar.CalendarScreen
import com.nexdark.ottero.feature.capture.CaptureKind
import com.nexdark.ottero.feature.capture.QuickCaptureSheet
import com.nexdark.ottero.feature.focus.FocusScreen
import com.nexdark.ottero.feature.focus.FocusTimerState
import com.nexdark.ottero.feature.focus.rememberFocusTimerState
import com.nexdark.ottero.feature.habits.HabitsScreen
import com.nexdark.ottero.feature.notes.NoteEditorScreen
import com.nexdark.ottero.feature.notes.NotesListScreen
import com.nexdark.ottero.feature.passwords.PasswordsScreen
import com.nexdark.ottero.feature.settings.SettingsScreen
import com.nexdark.ottero.feature.tasks.TasksScreen
import com.nexdark.ottero.feature.today.TodayScreen
import com.nexdark.ottero.feature.trash.TrashScreen
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import kotlinx.coroutines.launch

private enum class Destination(
    val label: String,
    val icon: ImageVector,
    val inBottomBar: Boolean,
) {
    Today("Today", Icons.Outlined.WbSunny, true),
    Notes("Notes", Icons.Outlined.Description, true),
    Tasks("Tasks", Icons.Outlined.CheckCircle, true),
    Habits("Habits", Icons.Outlined.Spa, true),
    Calendar("Calendar", Icons.Outlined.CalendarToday, false),
    Focus("Focus", Icons.Outlined.Timer, false),
    Passwords("Passwords", Icons.Outlined.Key, false),
    Trash("Trash", Icons.Outlined.DeleteOutline, false),
}

private val BottomDestinations = Destination.entries.filter { it.inBottomBar }
private val DrawerSecondaryDestinations = Destination.entries.filter { !it.inBottomBar }

@Composable
fun OtteroApp(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    onUpdatePrefs: (OtteroPrefsState) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val prefs = LocalOtteroPrefs.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val initialDestination = remember {
        when (prefs.startupDestination) {
            StartupDestination.Today -> Destination.Today
            StartupDestination.Notes -> Destination.Notes
            StartupDestination.Tasks -> Destination.Tasks
            StartupDestination.Habits -> Destination.Habits
            StartupDestination.Calendar -> Destination.Calendar
        }
    }
    var destination by rememberSaveable { mutableStateOf(initialDestination) }
    var activeNoteId by rememberSaveable { mutableStateOf<String?>(null) }
    var activeNoteIsNew by rememberSaveable { mutableStateOf(false) }
    var showSettings by rememberSaveable { mutableStateOf(false) }
    var captureSheetKind by rememberSaveable { mutableStateOf<CaptureKind?>(null) }
    var isCreatingNote by remember { mutableStateOf(false) }
    val focusTimer = rememberFocusTimerState()

    // Lock vault and pause realtime WS whenever the app is backgrounded.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) {
        engine.lockVaultNow()
        engine.realtime.stop()
    }

    // Re-unlock account-linked vault offline, connect live WS, and sync on foreground.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        engine.realtime.start(context)
        scope.launch {
            runSuspendCatching { engine.vaultUnlockWithAccount() }
            val status = runSuspendCatching { engine.accountStatus() }.getOrNull()
            if (status?.signedIn == true) {
                runSuspendCatching { engine.requestCoalescedSync() }
            }
        }
    }

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
            onUpdatePrefs = onUpdatePrefs,
            onBack = { showSettings = false },
        )
        return
    }

    val density = LocalDensity.current
    val windowWidthDp = with(density) { LocalWindowInfo.current.containerSize.width.toDp() }
    val useRail = windowWidthDp >= 600.dp

    val showFab = destination in listOf(
        Destination.Today,
        Destination.Notes,
        Destination.Tasks,
        Destination.Habits,
    )

    val fab: @Composable () -> Unit = {
        if (showFab) {
            FloatingActionButton(
                onClick = {
                    when (destination) {
                        Destination.Notes -> createAndOpenNote()
                        Destination.Today -> captureSheetKind = CaptureKind.Task
                        Destination.Tasks -> captureSheetKind = CaptureKind.Task
                        Destination.Habits -> captureSheetKind = CaptureKind.Habit
                        else -> Unit
                    }
                },
                containerColor = palette.ink,
                contentColor = palette.card,
            ) {
                Icon(
                    imageVector = Icons.Outlined.Add,
                    contentDescription = if (destination == Destination.Notes) "New note" else "New",
                )
            }
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
        BackHandler(enabled = destination != Destination.Today) {
            destination = Destination.Today
        }
        Row(modifier = Modifier.fillMaxSize()) {
            NavigationRail(
                containerColor = palette.recessed,
                header = {
                    Box(
                        modifier = Modifier
                            .padding(top = 12.dp, bottom = 8.dp)
                            .size(34.dp)
                            .clip(RoundedCornerShape(9.dp)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Image(
                            painter = painterResource(id = R.drawable.ic_launcher_foreground),
                            contentDescription = "Ottero",
                            contentScale = ContentScale.Crop,
                            modifier = Modifier
                                .fillMaxSize()
                                .graphicsLayer(scaleX = 1.48f, scaleY = 1.48f),
                        )
                    }
                },
            ) {
                Destination.entries.forEach { dest ->
                    NavigationRailItem(
                        selected = destination == dest,
                        onClick = { destination = dest },
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
                        title = destination.label,
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
                        DestinationContent(
                            destination = destination,
                            engine = engine,
                            focusTimer = focusTimer,
                            contentPadding = innerPadding,
                            onSelectDestination = { destination = it },
                            onOpenNote = { id ->
                                activeNoteIsNew = false
                                activeNoteId = id
                            },
                            onCreateNote = ::createAndOpenNote,
                            onOpenQuickCapture = { captureSheetKind = it },
                            onOpenSettings = { showSettings = true },
                        )
                    }
                }
            }
        }
    } else {
        val drawerState = rememberDrawerState(initialValue = DrawerValue.Closed)

        BackHandler(enabled = drawerState.isOpen || destination != Destination.Today) {
            if (drawerState.isOpen) {
                scope.launch { drawerState.close() }
            } else {
                destination = Destination.Today
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

                        BottomDestinations.forEach { dest ->
                            DrawerRow(
                                dest = dest,
                                selected = destination == dest,
                                onClick = {
                                    destination = dest
                                    scope.launch { drawerState.close() }
                                },
                            )
                        }

                        Spacer(Modifier.height(8.dp))
                        HorizontalDivider(color = palette.border)
                        Spacer(Modifier.height(8.dp))

                        DrawerSecondaryDestinations.forEach { dest ->
                            DrawerRow(
                                dest = dest,
                                selected = destination == dest,
                                onClick = {
                                    destination = dest
                                    scope.launch { drawerState.close() }
                                },
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
                        title = destination.label,
                        showDrawerButton = true,
                        onOpenDrawer = { scope.launch { drawerState.open() } },
                    )
                },
                snackbarHost = { OtteroSnackbarHost(feedback) },
                floatingActionButton = fab,
                bottomBar = {
                    NavigationBar(containerColor = palette.recessed) {
                        BottomDestinations.forEach { dest ->
                            NavigationBarItem(
                                selected = destination == dest,
                                onClick = { destination = dest },
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
                    DestinationContent(
                        destination = destination,
                        engine = engine,
                        focusTimer = focusTimer,
                        contentPadding = innerPadding,
                        onSelectDestination = { destination = it },
                        onOpenNote = { id ->
                            activeNoteIsNew = false
                            activeNoteId = id
                        },
                        onCreateNote = ::createAndOpenNote,
                        onOpenQuickCapture = { captureSheetKind = it },
                        onOpenSettings = { showSettings = true },
                    )
                }
            }
        }
    }
}

@Composable
private fun DrawerRow(
    dest: Destination,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    NavigationDrawerItem(
        label = { Text(dest.label) },
        icon = { Icon(dest.icon, contentDescription = null) },
        selected = selected,
        onClick = onClick,
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

@Composable
private fun DestinationContent(
    destination: Destination,
    engine: OtteroEngine,
    focusTimer: FocusTimerState,
    contentPadding: PaddingValues,
    onSelectDestination: (Destination) -> Unit,
    onOpenNote: (String) -> Unit,
    onCreateNote: () -> Unit,
    onOpenQuickCapture: (CaptureKind) -> Unit,
    onOpenSettings: () -> Unit,
) {
    when (destination) {
        Destination.Today -> TodayScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenTasks = { onSelectDestination(Destination.Tasks) },
            onOpenNotes = { onSelectDestination(Destination.Notes) },
            onOpenHabits = { onSelectDestination(Destination.Habits) },
            onOpenCalendar = { onSelectDestination(Destination.Calendar) },
            onOpenNote = onOpenNote,
            onCreateNote = onCreateNote,
            onAddTask = { onOpenQuickCapture(CaptureKind.Task) },
        )

        Destination.Notes -> NotesListScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenNote = onOpenNote,
            onCreateNote = onCreateNote,
        )

        Destination.Tasks -> TasksScreen(
            engine = engine,
            contentPadding = contentPadding,
        )

        Destination.Habits -> HabitsScreen(
            engine = engine,
            contentPadding = contentPadding,
        )

        Destination.Calendar -> CalendarScreen(
            engine = engine,
            contentPadding = contentPadding,
        )

        Destination.Focus -> FocusScreen(
            timer = focusTimer,
            contentPadding = contentPadding,
        )

        Destination.Passwords -> PasswordsScreen(
            engine = engine,
            contentPadding = contentPadding,
            onOpenSettings = onOpenSettings,
        )

        Destination.Trash -> TrashScreen(
            engine = engine,
            contentPadding = contentPadding,
        )
    }
}

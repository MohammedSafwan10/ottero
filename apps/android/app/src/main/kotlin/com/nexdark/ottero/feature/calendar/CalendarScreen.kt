package com.nexdark.ottero.feature.calendar

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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CalendarToday
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.components.OtteroPullToRefresh
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.prefs.LocalOtteroPrefs
import com.nexdark.ottero.core.prefs.formatDisplayTime
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.CalendarEventItem
import uniffi.ottero_mobile_ffi.HolidayItem
import java.time.DayOfWeek
import java.time.LocalDate
import java.time.YearMonth
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.time.temporal.TemporalAdjusters
import java.util.Locale

@Composable
fun CalendarScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val prefs = LocalOtteroPrefs.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()

    val today = remember { LocalDate.now() }
    var selectedIso by rememberSaveable { mutableStateOf(today.toString()) }
    var monthMode by rememberSaveable { mutableStateOf(false) }
    var draft by rememberSaveable { mutableStateOf("") }
    var events by remember { mutableStateOf<List<CalendarEventItem>>(emptyList()) }
    var holidays by remember { mutableStateOf<List<HolidayItem>>(emptyList()) }
    var editingEvent by remember { mutableStateOf<CalendarEventItem?>(null) }
    var isLoading by remember { mutableStateOf(true) }

    val selectedDate = remember(selectedIso) {
        runCatching { LocalDate.parse(selectedIso) }.getOrDefault(today)
    }
    val firstDayOfWeek = remember(prefs.weekStart) { prefs.weekStart.resolve() }
    val weekStart = remember(selectedDate, firstDayOfWeek) {
        selectedDate.with(TemporalAdjusters.previousOrSame(firstDayOfWeek))
    }
    val weekDays = remember(weekStart) {
        (0L..6L).map { weekStart.plusDays(it) }
    }
    val monthGridWeeks = remember(selectedDate.year, selectedDate.monthValue, firstDayOfWeek) {
        val ym = YearMonth.of(selectedDate.year, selectedDate.month)
        val gridStart = ym.atDay(1).with(TemporalAdjusters.previousOrSame(firstDayOfWeek))
        val gridEnd = ym.atEndOfMonth().with(TemporalAdjusters.nextOrSame(firstDayOfWeek.plus(6)))
        buildList {
            var cursor = gridStart
            while (!cursor.isAfter(gridEnd)) {
                add((0L..6L).map { cursor.plusDays(it) })
                cursor = cursor.plusWeeks(1)
            }
        }
    }
    val monthLabel = remember(selectedDate) {
        selectedDate.format(DateTimeFormatter.ofPattern("MMMM yyyy"))
    }

    fun submitQuickAdd() {
        val text = draft.trim()
        if (text.isEmpty()) return
        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        scope.launch {
            runSuspendCatching { engine.quickAddEvent(text, selectedIso) }
                .onSuccess { draft = "" }
                .onFailure { feedback.error(it.message ?: "Could not add event") }
        }
    }

    LaunchedEffect(engine, prefs.showHolidays) {
        combine(
            snapshotFlow { selectedIso },
            engine.changes.onStart { emit(Unit) },
        ) { dayIso, _ -> dayIso }.collectLatest { dayIso ->
            runSuspendCatching {
                val loadedEvents = engine.listEventsInRange(dayIso, dayIso)
                val loadedHolidays = if (prefs.showHolidays) {
                    engine.listHolidaysInRange(dayIso, dayIso)
                } else {
                    emptyList()
                }
                loadedEvents to loadedHolidays
            }.onSuccess { (loadedEvents, loadedHolidays) ->
                events = loadedEvents
                holidays = loadedHolidays
                isLoading = false
            }.onFailure {
                isLoading = false
                feedback.error(it.message ?: "Could not load events")
            }
        }
    }

    val activeEvent = editingEvent
    if (activeEvent != null) {
        EventDetailSheet(
            engine = engine,
            event = activeEvent,
            onDismiss = { editingEvent = null },
            onTrashed = { trashedId ->
                editingEvent = null
                feedback.undo("Event moved to trash") { engine.restoreEvent(trashedId) }
            },
        )
    }

    OtteroPullToRefresh(engine = engine, modifier = Modifier.fillMaxSize()) {
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
            item(key = "calendar-header", contentType = "calendar_grid") {
                OtteroCard {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Text(
                            text = monthLabel,
                            style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold),
                            color = palette.ink,
                        )
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            TextButton(onClick = { monthMode = !monthMode }) {
                                Text(if (monthMode) "Week" else "Month", color = palette.muted)
                            }
                            if (selectedDate != today) {
                                TextButton(onClick = { selectedIso = today.toString() }) {
                                    Text("Today", color = palette.muted)
                                }
                            }
                            IconButton(
                                onClick = {
                                    selectedIso = if (monthMode) {
                                        selectedDate.minusMonths(1).toString()
                                    } else {
                                        selectedDate.minusWeeks(1).toString()
                                    }
                                },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Outlined.KeyboardArrowLeft,
                                    contentDescription = if (monthMode) "Previous month" else "Previous week",
                                    tint = palette.muted,
                                )
                            }
                            IconButton(
                                onClick = {
                                    selectedIso = if (monthMode) {
                                        selectedDate.plusMonths(1).toString()
                                    } else {
                                        selectedDate.plusWeeks(1).toString()
                                    }
                                },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                                    contentDescription = if (monthMode) "Next month" else "Next week",
                                    tint = palette.muted,
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(10.dp))

                    if (monthMode) {
                        val headerDays = remember(firstDayOfWeek) {
                            (0L..6L).map { DayOfWeek.of(((firstDayOfWeek.value - 1 + it.toInt()) % 7) + 1) }
                        }
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            headerDays.forEach { dow ->
                                Text(
                                    text = dow.getDisplayName(TextStyle.NARROW, Locale.getDefault()),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = palette.muted,
                                    modifier = Modifier.weight(1f),
                                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                )
                            }
                        }
                        Spacer(Modifier.height(6.dp))
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            monthGridWeeks.forEach { rowDays ->
                                Row(
                                    modifier = Modifier.fillMaxWidth(),
                                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                                ) {
                                    rowDays.forEach { day ->
                                        val isSelected = day == selectedDate
                                        val isToday = day == today
                                        val inMonth = day.month == selectedDate.month
                                        Surface(
                                            modifier = Modifier
                                                .weight(1f)
                                                .clip(RoundedCornerShape(8.dp))
                                                .clickable { selectedIso = day.toString() },
                                            shape = RoundedCornerShape(8.dp),
                                            color = if (isSelected) palette.ink else palette.recessed.copy(alpha = if (inMonth) 1f else 0.4f),
                                            border = if (isToday && !isSelected) {
                                                BorderStroke(1.dp, palette.ink.copy(alpha = 0.4f))
                                            } else {
                                                null
                                            },
                                        ) {
                                            Text(
                                                text = day.dayOfMonth.toString(),
                                                style = MaterialTheme.typography.labelMedium.copy(
                                                    fontWeight = if (isSelected || isToday) FontWeight.SemiBold else FontWeight.Normal,
                                                ),
                                                color = when {
                                                    isSelected -> palette.card
                                                    inMonth -> palette.ink
                                                    else -> palette.muted.copy(alpha = 0.55f)
                                                },
                                                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                                modifier = Modifier.padding(vertical = 8.dp),
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            weekDays.forEach { day ->
                                val isSelected = day == selectedDate
                                val isToday = day == today
                                Surface(
                                    modifier = Modifier
                                        .weight(1f)
                                        .clip(RoundedCornerShape(10.dp))
                                        .clickable { selectedIso = day.toString() },
                                    shape = RoundedCornerShape(10.dp),
                                    color = if (isSelected) palette.ink else palette.recessed,
                                    border = if (isToday && !isSelected) {
                                        BorderStroke(1.dp, palette.ink.copy(alpha = 0.4f))
                                    } else {
                                        null
                                    },
                                ) {
                                    Column(
                                        modifier = Modifier.padding(vertical = 8.dp),
                                        horizontalAlignment = Alignment.CenterHorizontally,
                                        verticalArrangement = Arrangement.spacedBy(2.dp),
                                    ) {
                                        Text(
                                            text = day.dayOfWeek.getDisplayName(TextStyle.NARROW, Locale.getDefault()),
                                            style = MaterialTheme.typography.labelSmall,
                                            color = if (isSelected) palette.card.copy(alpha = 0.8f) else palette.muted,
                                        )
                                        Text(
                                            text = day.dayOfMonth.toString(),
                                            style = MaterialTheme.typography.labelLarge.copy(
                                                fontWeight = if (isSelected || isToday) FontWeight.SemiBold else FontWeight.Normal,
                                            ),
                                            color = if (isSelected) palette.card else palette.ink,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if (holidays.isNotEmpty()) {
                items(
                    items = holidays,
                    key = { "holiday-${it.dateIso}-${it.name}" },
                    contentType = { "holiday_card" },
                ) { holiday ->
                    OtteroCard {
                        Text(
                            text = holiday.name,
                            style = MaterialTheme.typography.bodyMedium,
                            color = palette.ink,
                        )
                        Text(
                            text = "Public holiday",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.muted,
                        )
                    }
                }
            }

            item(key = "calendar-quick-add", contentType = "quick_add") {
                OutlinedTextField(
                    value = draft,
                    onValueChange = { draft = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = { Text("Add an event", color = palette.muted) },
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

            if (!isLoading && events.isEmpty() && holidays.isEmpty()) {
                item(key = "calendar-empty", contentType = "empty") {
                    OtteroCard {
                        EmptyPlaceholder(
                            icon = Icons.Outlined.CalendarToday,
                            title = "No events",
                            hint = "Add an event above.",
                        )
                    }
                }
            } else {
                items(
                    items = events,
                    key = { "event-${it.id}-${it.dateIso}-${it.startTime ?: "all"}" },
                    contentType = { "event_card" },
                ) { event ->
                    OtteroCard(modifier = Modifier.clickable { editingEvent = event }) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = event.title,
                                    style = MaterialTheme.typography.bodyLarge,
                                    color = palette.ink,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                val timeLabel = if (event.allDay) {
                                    "All day"
                                } else {
                                    listOfNotNull(
                                        event.startTime?.let { formatDisplayTime(it, prefs.use24HourTime) },
                                        event.endTime?.let { formatDisplayTime(it, prefs.use24HourTime) },
                                    ).joinToString(" – ")
                                }
                                Text(
                                    text = if (event.location.isNotBlank()) "$timeLabel · ${event.location}" else timeLabel,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = palette.muted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                            Spacer(Modifier.width(8.dp))
                            IconButton(
                                onClick = {
                                    scope.launch {
                                        runSuspendCatching { engine.trashEvent(event.id) }
                                            .onSuccess {
                                                feedback.undo("Event moved to trash") {
                                                    engine.restoreEvent(event.id)
                                                }
                                            }
                                            .onFailure { feedback.error(it.message ?: "Could not delete event") }
                                    }
                                },
                                modifier = Modifier.size(44.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.Outlined.DeleteOutline,
                                    contentDescription = "Move event to trash",
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
}

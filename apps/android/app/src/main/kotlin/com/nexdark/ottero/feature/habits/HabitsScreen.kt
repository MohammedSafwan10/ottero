package com.nexdark.ottero.feature.habits

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Spa
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.HabitItem

@Composable
fun HabitsScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()

    var draft by rememberSaveable { mutableStateOf("") }
    var habits by remember { mutableStateOf<List<HabitItem>>(emptyList()) }
    var isLoading by remember { mutableStateOf(true) }

    fun submitQuickAdd() {
        val text = draft.trim()
        if (text.isEmpty()) return
        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        scope.launch {
            runSuspendCatching { engine.quickAddHabit(text) }
                .onSuccess { draft = "" }
                .onFailure { feedback.error(it.message ?: "Could not add habit") }
        }
    }

    LaunchedEffect(engine) {
        engine.changes.onStart { emit(Unit) }.collectLatest {
            runSuspendCatching { engine.listHabits() }
                .onSuccess {
                    habits = it
                    isLoading = false
                }
                .onFailure {
                    isLoading = false
                    feedback.error(it.message ?: "Could not load habits")
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
        item(key = "habits-quick-add", contentType = "quick_add") {
            OutlinedTextField(
                value = draft,
                onValueChange = { draft = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Add a habit", color = palette.muted) },
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

        if (!isLoading && habits.isEmpty()) {
            item(key = "habits-empty", contentType = "empty") {
                OtteroCard {
                    EmptyPlaceholder(
                        icon = Icons.Outlined.Spa,
                        title = "No habits yet",
                        hint = "Add a habit above.",
                    )
                }
            }
        } else {
            items(
                items = habits,
                key = { "habit-${it.id}" },
                contentType = { "habit_card" },
            ) { habit ->
                Surface(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable {
                            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            scope.launch {
                                runSuspendCatching { engine.tapHabitToday(habit.id) }
                                    .onFailure { feedback.error(it.message ?: "Could not log habit") }
                            }
                        },
                    shape = RoundedCornerShape(14.dp),
                    color = if (habit.doneToday) palette.success.copy(alpha = 0.14f) else palette.card,
                    border = BorderStroke(
                        1.dp,
                        if (habit.doneToday) palette.success else palette.border,
                    ),
                ) {
                    Row(
                        modifier = Modifier
                            .heightIn(min = 56.dp)
                            .padding(horizontal = 16.dp, vertical = 12.dp),
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
                                modifier = Modifier.size(20.dp),
                            )
                            Spacer(Modifier.width(12.dp))
                            Text(
                                text = habit.name,
                                style = MaterialTheme.typography.bodyLarge,
                                color = palette.ink,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Text(
                            text = "${habit.todayCount}/${habit.targetPerDay}",
                            style = MaterialTheme.typography.labelLarge,
                            color = if (habit.doneToday) palette.success else palette.muted,
                        )
                    }
                }
            }
        }
    }
}

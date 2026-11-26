package com.nexdark.ottero.feature.habits

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.HabitItem

private val ScheduleOptions = listOf(
    "daily" to "Daily",
    "weekdays" to "Weekdays",
    "weekly:3" to "3x/week",
    "weekly:5" to "5x/week",
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HabitDetailSheet(
    engine: OtteroEngine,
    habit: HabitItem,
    onDismiss: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var name by remember(habit.id) { mutableStateOf(habit.name) }
    var scheduleCode by remember(habit.id) { mutableStateOf(habit.scheduleCode) }
    var target by remember(habit.id) { mutableIntStateOf(habit.targetPerDay.toInt().coerceAtLeast(1)) }
    var busy by remember { mutableStateOf(false) }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = palette.canvas,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = "Edit habit",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )

            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Name") },
                singleLine = true,
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

            Text(
                text = "Schedule",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ScheduleOptions.forEach { (code, label) ->
                    FilterChip(
                        selected = scheduleCode == code,
                        onClick = { scheduleCode = code },
                        label = { Text(label) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.recessed,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }

            Text(
                text = "Daily target",
                style = MaterialTheme.typography.labelMedium,
                color = palette.muted,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(1, 2, 3, 5, 8).forEach { count ->
                    FilterChip(
                        selected = target == count,
                        onClick = { target = count },
                        label = { Text("${count}x") },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = palette.ink,
                            selectedLabelColor = palette.card,
                            containerColor = palette.recessed,
                            labelColor = palette.muted,
                        ),
                    )
                }
            }

            if (habit.bestStreak > 0u) {
                Text(
                    text = "Current streak: ${habit.currentStreak}d · Best: ${habit.bestStreak}d",
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.muted,
                )
            }

            Spacer(Modifier.height(4.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    enabled = !busy,
                    onClick = {
                        feedback.confirmDestructive(
                            title = "Delete habit?",
                            message = "\"${habit.name}\" and its check-in history will be removed.",
                            confirmLabel = "Delete",
                        ) {
                            engine.deleteHabit(habit.id)
                            onDismiss()
                        }
                    },
                    shape = RoundedCornerShape(10.dp),
                ) {
                    Text("Delete", color = palette.danger)
                }
                OutlinedButton(
                    enabled = !busy,
                    onClick = {
                        busy = true
                        scope.launch {
                            runSuspendCatching { engine.archiveHabit(habit.id) }
                                .onSuccess {
                                    feedback.info("Habit archived")
                                    onDismiss()
                                }
                                .onFailure {
                                    busy = false
                                    feedback.error(it.message ?: "Could not archive habit")
                                }
                        }
                    },
                    shape = RoundedCornerShape(10.dp),
                ) {
                    Text("Archive", color = palette.muted)
                }
                Button(
                    enabled = !busy && name.isNotBlank(),
                    onClick = {
                        busy = true
                        scope.launch {
                            runSuspendCatching {
                                engine.updateHabit(habit.id, name.trim(), scheduleCode, target)
                            }.onSuccess {
                                onDismiss()
                            }.onFailure {
                                busy = false
                                feedback.error(it.message ?: "Could not save habit")
                            }
                        }
                    },
                    modifier = Modifier.weight(1f),
                    shape = RoundedCornerShape(10.dp),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = palette.ink,
                        contentColor = palette.card,
                    ),
                ) {
                    Text("Save")
                }
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}

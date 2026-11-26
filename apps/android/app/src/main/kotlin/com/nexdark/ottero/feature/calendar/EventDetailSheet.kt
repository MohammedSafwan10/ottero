package com.nexdark.ottero.feature.calendar

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.CalendarEventItem

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EventDetailSheet(
    engine: OtteroEngine,
    event: CalendarEventItem,
    onDismiss: () -> Unit,
    onTrashed: (String) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    var title by remember(event.id) { mutableStateOf(event.title) }
    var dateIso by remember(event.id) { mutableStateOf(event.dateIso) }
    var allDay by remember(event.id) { mutableStateOf(event.allDay) }
    var startTime by remember(event.id) { mutableStateOf(event.startTime ?: "09:00") }
    var endTime by remember(event.id) { mutableStateOf(event.endTime ?: "10:00") }
    var location by remember(event.id) { mutableStateOf(event.location) }
    var notes by remember(event.id) { mutableStateOf(event.notes) }
    var busy by remember { mutableStateOf(false) }

    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedContainerColor = palette.card,
        unfocusedContainerColor = palette.card,
        focusedBorderColor = palette.ink,
        unfocusedBorderColor = palette.border,
        focusedTextColor = palette.ink,
        unfocusedTextColor = palette.ink,
    )

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = palette.canvas,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = "Edit event",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )

            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Title") },
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            OutlinedTextField(
                value = dateIso,
                onValueChange = { dateIso = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Date (YYYY-MM-DD)") },
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "All day",
                    style = MaterialTheme.typography.bodyMedium,
                    color = palette.ink,
                )
                Switch(
                    checked = allDay,
                    onCheckedChange = { allDay = it },
                )
            }

            if (!allDay) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    OutlinedTextField(
                        value = startTime,
                        onValueChange = { startTime = it },
                        modifier = Modifier.weight(1f),
                        label = { Text("Start (HH:MM)") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )
                    OutlinedTextField(
                        value = endTime,
                        onValueChange = { endTime = it },
                        modifier = Modifier.weight(1f),
                        label = { Text("End (HH:MM)") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )
                }
            }

            OutlinedTextField(
                value = location,
                onValueChange = { location = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Location") },
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Notes") },
                minLines = 2,
                maxLines = 4,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            Spacer(Modifier.height(4.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                OutlinedButton(
                    enabled = !busy,
                    onClick = {
                        busy = true
                        scope.launch {
                            runSuspendCatching { engine.trashEvent(event.id) }
                                .onSuccess { onTrashed(event.id) }
                                .onFailure {
                                    busy = false
                                    feedback.error(it.message ?: "Could not delete event")
                                }
                        }
                    },
                    shape = RoundedCornerShape(10.dp),
                ) {
                    Text("Delete", color = palette.danger)
                }
                Button(
                    enabled = !busy && title.isNotBlank(),
                    onClick = {
                        busy = true
                        scope.launch {
                            runSuspendCatching {
                                engine.updateEvent(
                                    id = event.id,
                                    title = title.trim(),
                                    notes = notes,
                                    location = location,
                                    dateIso = dateIso.trim(),
                                    allDay = allDay,
                                    startTime = if (allDay) null else startTime.trim(),
                                    endTime = if (allDay) null else endTime.trim(),
                                )
                            }
                                .onSuccess { onDismiss() }
                                .onFailure {
                                    busy = false
                                    feedback.error(it.message ?: "Could not save event")
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

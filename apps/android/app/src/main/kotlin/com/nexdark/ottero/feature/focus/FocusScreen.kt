package com.nexdark.ottero.feature.focus

import android.os.SystemClock
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.components.SectionHeader
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import uniffi.ottero_mobile_ffi.FocusSessionItem
import java.util.Locale

enum class FocusPreset(val label: String, val code: String, val seconds: Int) {
    Focus("Focus", "focus", 25 * 60),
    ShortBreak("Short break", "short_break", 5 * 60),
    LongBreak("Long break", "long_break", 15 * 60),
}

/**
 * Wall-clock backed focus timer state hoisted above tab navigation so switching
 * tabs or backgrounding the app never drifts or resets an active session.
 */
@Stable
class FocusTimerState {
    var preset by mutableStateOf(FocusPreset.Focus)
        private set
    var remainingSeconds by mutableIntStateOf(FocusPreset.Focus.seconds)
        private set
    var running by mutableStateOf(false)
        private set
    private var endElapsedMs by mutableLongStateOf(0L)

    fun selectPreset(next: FocusPreset) {
        running = false
        preset = next
        remainingSeconds = next.seconds
        endElapsedMs = 0L
    }

    fun toggle() {
        if (running) {
            syncNow()
            running = false
            endElapsedMs = 0L
        } else {
            if (remainingSeconds <= 0) {
                remainingSeconds = preset.seconds
            }
            endElapsedMs = SystemClock.elapsedRealtime() + remainingSeconds * 1000L
            running = true
        }
    }

    fun reset() {
        running = false
        remainingSeconds = preset.seconds
        endElapsedMs = 0L
    }

    /** Returns true if the timer just finished on this tick. */
    fun syncNow(): Boolean {
        if (!running) return false
        val leftMs = (endElapsedMs - SystemClock.elapsedRealtime()).coerceAtLeast(0L)
        remainingSeconds = ((leftMs + 999L) / 1000L).toInt()
        if (leftMs == 0L) {
            running = false
            endElapsedMs = 0L
            return true
        }
        return false
    }
}

@Composable
fun rememberFocusTimerState(): FocusTimerState = remember { FocusTimerState() }

@Composable
fun FocusScreen(
    engine: OtteroEngine,
    timer: FocusTimerState,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current
    var sessions by remember { mutableStateOf<List<FocusSessionItem>>(emptyList()) }

    LaunchedEffect(engine) {
        engine.changes.onStart { emit(Unit) }.collectLatest {
            sessions = runSuspendCatching { engine.listTodayFocusSessions() }.getOrDefault(emptyList())
        }
    }

    LaunchedEffect(timer.running) {
        if (!timer.running) return@LaunchedEffect
        while (timer.running) {
            if (timer.syncNow()) {
                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                val finishedPreset = timer.preset
                runSuspendCatching {
                    engine.recordFocusSession(
                        label = finishedPreset.label,
                        kindCode = finishedPreset.code,
                        plannedSecs = finishedPreset.seconds,
                        actualSecs = finishedPreset.seconds,
                        completed = true,
                    )
                }
                feedback.info("${finishedPreset.label} finished and saved")
                break
            }
            delay(250L)
        }
    }

    val total = timer.preset.seconds.coerceAtLeast(1)
    val progress = (timer.remainingSeconds.toFloat() / total.toFloat()).coerceIn(0f, 1f)
    val mins = timer.remainingSeconds / 60
    val secs = timer.remainingSeconds % 60
    val timeText = String.format(Locale.US, "%02d:%02d", mins, secs)
    val focusTotalMins = sessions.filter { it.kind == "focus" }.sumOf { it.actualSecs.toInt() } / 60

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(
                start = contentPadding.calculateStartPadding(layoutDirection) + 20.dp,
                end = contentPadding.calculateEndPadding(layoutDirection) + 20.dp,
                top = contentPadding.calculateTopPadding() + 16.dp,
                bottom = contentPadding.calculateBottomPadding() + 32.dp,
            ),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        ) {
            FocusPreset.entries.forEach { item ->
                FilterChip(
                    selected = timer.preset == item,
                    onClick = { timer.selectPreset(item) },
                    label = { Text(item.label) },
                    colors = FilterChipDefaults.filterChipColors(
                        selectedContainerColor = palette.ink,
                        selectedLabelColor = palette.card,
                        containerColor = palette.card,
                        labelColor = palette.muted,
                    ),
                )
            }
        }

        Box(
            modifier = Modifier.size(230.dp),
            contentAlignment = Alignment.Center,
        ) {
            val trackColor = palette.border
            val progressColor = if (timer.preset == FocusPreset.Focus) palette.ink else palette.success
            Canvas(modifier = Modifier.fillMaxSize()) {
                val stroke = Stroke(width = 8.dp.toPx(), cap = StrokeCap.Round)
                drawArc(
                    color = trackColor,
                    startAngle = -90f,
                    sweepAngle = 360f,
                    useCenter = false,
                    style = stroke,
                )
                drawArc(
                    color = progressColor,
                    startAngle = -90f,
                    sweepAngle = 360f * progress,
                    useCenter = false,
                    style = stroke,
                )
            }
            Text(
                text = timeText,
                style = MaterialTheme.typography.displayMedium.copy(
                    fontSize = 50.sp,
                    fontWeight = FontWeight.Medium,
                    fontFeatureSettings = "tnum",
                ),
                color = palette.ink,
            )
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
        ) {
            Button(
                onClick = timer::toggle,
                modifier = Modifier
                    .height(50.dp)
                    .width(148.dp),
                shape = RoundedCornerShape(14.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(
                    text = if (timer.running) "Pause" else "Start",
                    style = MaterialTheme.typography.labelLarge,
                )
            }

            Spacer(Modifier.width(12.dp))

            OutlinedButton(
                onClick = timer::reset,
                modifier = Modifier.height(50.dp),
                shape = RoundedCornerShape(14.dp),
            ) {
                Text(
                    text = "Reset",
                    style = MaterialTheme.typography.labelLarge,
                    color = palette.ink,
                )
            }
        }

        if (sessions.isNotEmpty()) {
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                SectionHeader(
                    title = "Today · ${focusTotalMins}m focused",
                )
                sessions.forEach { session ->
                    OtteroCard {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = session.label,
                                style = MaterialTheme.typography.bodyMedium,
                                color = palette.ink,
                            )
                            Text(
                                text = "${(session.actualSecs.toInt() / 60).coerceAtLeast(1)}m",
                                style = MaterialTheme.typography.labelMedium,
                                color = palette.muted,
                            )
                        }
                    }
                }
            }
        }
    }
}

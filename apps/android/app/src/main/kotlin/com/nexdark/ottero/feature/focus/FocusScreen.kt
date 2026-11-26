package com.nexdark.ottero.feature.focus

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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
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
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.delay
import java.util.Locale

private enum class FocusPreset(val label: String, val seconds: Int) {
    Focus("Focus", 25 * 60),
    ShortBreak("Short break", 5 * 60),
    LongBreak("Long break", 15 * 60),
}

@Composable
fun FocusScreen(
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val haptic = LocalHapticFeedback.current
    val layoutDirection = LocalLayoutDirection.current

    var preset by rememberSaveable { mutableStateOf(FocusPreset.Focus) }
    var remainingSeconds by rememberSaveable { mutableIntStateOf(FocusPreset.Focus.seconds) }
    var running by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(running, preset) {
        if (!running) return@LaunchedEffect
        while (running && remainingSeconds > 0) {
            delay(1000L)
            remainingSeconds -= 1
        }
        if (running && remainingSeconds <= 0) {
            running = false
            haptic.performHapticFeedback(HapticFeedbackType.LongPress)
            feedback.info("${preset.label} finished")
        }
    }

    val total = preset.seconds.coerceAtLeast(1)
    val progress = (remainingSeconds.toFloat() / total.toFloat()).coerceIn(0f, 1f)
    val mins = remainingSeconds / 60
    val secs = remainingSeconds % 60
    val timeText = String.format(Locale.US, "%02d:%02d", mins, secs)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(
                start = contentPadding.calculateStartPadding(layoutDirection) + 20.dp,
                end = contentPadding.calculateEndPadding(layoutDirection) + 20.dp,
                top = contentPadding.calculateTopPadding() + 16.dp,
                bottom = contentPadding.calculateBottomPadding() + 32.dp,
            ),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        ) {
            FocusPreset.entries.forEach { item ->
                FilterChip(
                    selected = preset == item,
                    onClick = {
                        running = false
                        preset = item
                        remainingSeconds = item.seconds
                    },
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
            modifier = Modifier.size(240.dp),
            contentAlignment = Alignment.Center,
        ) {
            val trackColor = palette.border
            val progressColor = palette.ink
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
                    fontSize = 52.sp,
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
                onClick = {
                    if (remainingSeconds <= 0) {
                        remainingSeconds = preset.seconds
                    }
                    running = !running
                },
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
                    text = if (running) "Pause" else "Start",
                    style = MaterialTheme.typography.labelLarge,
                )
            }

            Spacer(Modifier.width(12.dp))

            OutlinedButton(
                onClick = {
                    running = false
                    remainingSeconds = preset.seconds
                },
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
    }
}

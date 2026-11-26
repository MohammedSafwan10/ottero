package com.nexdark.ottero.feature.onboarding

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.R
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import kotlinx.coroutines.launch

@Composable
fun OnboardingScreen(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    onComplete: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val scope = rememberCoroutineScope()
    var isSettingUp by remember { mutableStateOf(false) }

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = palette.canvas,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .systemBarsPadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 24.dp, vertical = 32.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                modifier = Modifier
                    .widthIn(max = 460.dp)
                    .fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                Surface(
                    shape = RoundedCornerShape(18.dp),
                    border = BorderStroke(1.dp, palette.border),
                    modifier = Modifier.size(64.dp),
                ) {
                    Image(
                        painter = painterResource(id = R.drawable.ic_launcher_foreground),
                        contentDescription = "Ottero",
                        modifier = Modifier
                            .fillMaxSize()
                            .clip(RoundedCornerShape(18.dp)),
                    )
                }

                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Welcome to Ottero",
                        style = MaterialTheme.typography.headlineMedium,
                        color = palette.ink,
                    )
                    Text(
                        text = "Your calm, local-first home for notes, tasks, and daily habits. Stored privately on this device.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = palette.muted,
                    )
                }

                OtteroCard {
                    Text(
                        text = "Appearance",
                        style = MaterialTheme.typography.titleSmall,
                        color = palette.ink,
                    )
                    Spacer(Modifier.height(10.dp))
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        ThemePreference.entries.forEach { pref ->
                            FilterChip(
                                selected = themePreference == pref,
                                onClick = { onThemeChange(pref) },
                                label = { Text(pref.name) },
                                colors = FilterChipDefaults.filterChipColors(
                                    selectedContainerColor = palette.ink,
                                    selectedLabelColor = palette.card,
                                    containerColor = palette.recessed,
                                    labelColor = palette.muted,
                                ),
                            )
                        }
                    }
                }

                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Button(
                        enabled = !isSettingUp,
                        onClick = {
                            isSettingUp = true
                            scope.launch {
                                runSuspendCatching {
                                    engine.createNote(
                                        title = "Welcome to Ottero",
                                        body = "Everything here is stored locally in SQLite and powered by Ottero's shared Rust engine.\n\nTry:\n• Adding a task with natural language like \"Review notes fri !high\"\n• Tracking a daily habit in the Habits tab\n• Swiping from the left edge to open your workspace sidebar",
                                    )
                                    engine.quickAddTask("Explore Today, Notes, Tasks, and Habits", true)
                                    engine.quickAddTask("Try quick-add with priority !high", true)
                                    engine.quickAddHabit("Read 20 pages")
                                }
                                onComplete()
                            }
                        },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(50.dp),
                        shape = RoundedCornerShape(12.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = palette.ink,
                            contentColor = palette.card,
                        ),
                    ) {
                        Text(
                            text = if (isSettingUp) "Setting up…" else "Start with sample workspace",
                            style = MaterialTheme.typography.labelLarge,
                        )
                    }

                    OutlinedButton(
                        enabled = !isSettingUp,
                        onClick = onComplete,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(50.dp),
                        shape = RoundedCornerShape(12.dp),
                        border = BorderStroke(1.dp, palette.border),
                    ) {
                        Text(
                            text = "Start blank",
                            style = MaterialTheme.typography.labelLarge,
                            color = palette.ink,
                        )
                    }
                }
            }
        }
    }
}

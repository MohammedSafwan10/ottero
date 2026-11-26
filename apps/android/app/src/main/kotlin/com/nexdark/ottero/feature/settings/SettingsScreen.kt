package com.nexdark.ottero.feature.settings

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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import uniffi.ottero_mobile_ffi.AppInfo

@Composable
fun SettingsScreen(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    contentPadding: PaddingValues,
) {
    val palette = LocalOtteroPalette.current
    val layoutDirection = LocalLayoutDirection.current
    var info by remember { mutableStateOf<AppInfo?>(null) }

    LaunchedEffect(engine) {
        info = runCatching { engine.appInfo() }.getOrNull()
    }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(
            start = contentPadding.calculateStartPadding(layoutDirection) + 16.dp,
            end = contentPadding.calculateEndPadding(layoutDirection) + 16.dp,
            top = contentPadding.calculateTopPadding() + 16.dp,
            bottom = contentPadding.calculateBottomPadding() + 32.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item(key = "settings-title", contentType = "header") {
            Text(
                text = "Settings",
                style = MaterialTheme.typography.headlineMedium,
                color = palette.ink,
            )
        }

        item(key = "appearance-card", contentType = "appearance") {
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
        }

        item(key = "about-card", contentType = "about") {
            OtteroCard {
                Text(
                    text = "About",
                    style = MaterialTheme.typography.titleSmall,
                    color = palette.ink,
                )
                Spacer(Modifier.height(8.dp))
                val loadedInfo = info
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        text = if (loadedInfo != null) "Ottero ${loadedInfo.version}" else "Ottero",
                        style = MaterialTheme.typography.bodyMedium,
                        color = palette.ink,
                    )
                    Text(
                        text = if (loadedInfo != null) {
                            "Local SQLite schema v${loadedInfo.schemaVersion}"
                        } else {
                            "Local SQLite store"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = palette.muted,
                    )
                }
            }
        }
    }
}

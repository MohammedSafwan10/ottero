package com.nexdark.ottero

import android.content.Context
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroConfirmDialogHost
import com.nexdark.ottero.core.feedback.rememberOtteroFeedback
import com.nexdark.ottero.core.security.AccountKeystore
import com.nexdark.ottero.feature.onboarding.OnboardingScreen
import com.nexdark.ottero.navigation.OtteroApp
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.OtteroTheme
import com.nexdark.ottero.ui.theme.ThemePreference

private const val PREFS_NAME = "ottero_prefs"
private const val KEY_THEME = "theme_preference"
private const val KEY_ONBOARDING_DONE = "onboarding_done"

private sealed interface EngineState {
    data object Loading : EngineState
    data class Ready(val engine: OtteroEngine) : EngineState
    data class Failed(val message: String) : EngineState
}

class MainActivity : ComponentActivity() {

    private fun prefs() = getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    private fun loadThemePreference(): ThemePreference {
        val raw = prefs().getString(KEY_THEME, null)
        return ThemePreference.entries.firstOrNull { it.name == raw } ?: ThemePreference.System
    }

    private fun saveThemePreference(pref: ThemePreference) {
        prefs().edit().putString(KEY_THEME, pref.name).apply()
    }

    private fun isOnboardingDone(): Boolean = prefs().getBoolean(KEY_ONBOARDING_DONE, false)

    private fun setOnboardingDone(done: Boolean) {
        prefs().edit().putBoolean(KEY_ONBOARDING_DONE, done).apply()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)

        val initialTheme = loadThemePreference()
        val initialOnboardingDone = isOnboardingDone()

        setContent {
            var themePreference by remember { mutableStateOf(initialTheme) }
            var onboardingDone by remember { mutableStateOf(initialOnboardingDone) }
            val isDark = when (themePreference) {
                ThemePreference.System -> isSystemInDarkTheme()
                ThemePreference.Light -> false
                ThemePreference.Dark -> true
            }

            DisposableEffect(isDark) {
                val style = if (isDark) {
                    SystemBarStyle.dark(Color.TRANSPARENT)
                } else {
                    SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
                }
                enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
                onDispose {}
            }

            OtteroTheme(preference = themePreference) {
                val feedback = rememberOtteroFeedback()
                CompositionLocalProvider(LocalOtteroFeedback provides feedback) {
                    var engineState by remember { mutableStateOf<EngineState>(EngineState.Loading) }
                    var retryCount by remember { mutableIntStateOf(0) }

                    LaunchedEffect(retryCount) {
                        engineState = EngineState.Loading
                        engineState = OtteroEngine.open(applicationContext).fold(
                            onSuccess = { opened ->
                                // Restore hardware-encrypted Ottero Cloud session if present.
                                AccountKeystore.load(applicationContext)?.let { saved ->
                                    runCatching { opened.accountRestoreSession(saved) }
                                }
                                EngineState.Ready(opened)
                            },
                            onFailure = {
                                EngineState.Failed(it.message ?: "Could not open local database")
                            },
                        )
                    }

                    when (val state = engineState) {
                        is EngineState.Loading -> {
                            Surface(
                                modifier = Modifier.fillMaxSize(),
                                color = LocalOtteroPalette.current.canvas,
                            ) {}
                        }

                        is EngineState.Failed -> {
                            EngineErrorScreen(
                                message = state.message,
                                onRetry = { retryCount++ },
                            )
                        }

                        is EngineState.Ready -> {
                            if (!onboardingDone) {
                                OnboardingScreen(
                                    themePreference = themePreference,
                                    onThemeChange = { next ->
                                        themePreference = next
                                        saveThemePreference(next)
                                    },
                                    onComplete = {
                                        onboardingDone = true
                                        setOnboardingDone(true)
                                    },
                                )
                            } else {
                                OtteroApp(
                                    engine = state.engine,
                                    themePreference = themePreference,
                                    onThemeChange = { next ->
                                        themePreference = next
                                        saveThemePreference(next)
                                    },
                                )
                            }
                        }
                    }

                    OtteroConfirmDialogHost(feedback = feedback)
                }
            }
        }
    }
}

@Composable
private fun EngineErrorScreen(
    message: String,
    onRetry: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = palette.canvas,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            contentAlignment = Alignment.Center,
        ) {
            OtteroCard {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = "Unable to open Ottero",
                        style = MaterialTheme.typography.titleMedium,
                        color = palette.ink,
                    )
                    Text(
                        text = message,
                        style = MaterialTheme.typography.bodyMedium,
                        color = palette.muted,
                    )
                    Spacer(Modifier.height(4.dp))
                    Button(
                        onClick = onRetry,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = palette.ink,
                            contentColor = palette.card,
                        ),
                    ) {
                        Text("Retry")
                    }
                }
            }
        }
    }
}

package com.nexdark.ottero.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

enum class ThemePreference { System, Light, Dark }

@Immutable
data class OtteroPalette(
    val canvas: Color,
    val recessed: Color,
    val card: Color,
    val ink: Color,
    val muted: Color,
    val border: Color,
    val success: Color,
    val warning: Color,
    val info: Color,
    val danger: Color,
    val isDark: Boolean,
)

private val LightPalette = OtteroPalette(
    canvas = AlabasterCanvas,
    recessed = AlabasterSurface,
    card = AlabasterCard,
    ink = AlabasterInk,
    muted = AlabasterMuted,
    border = AlabasterBorder,
    success = AlabasterSage,
    warning = AlabasterOchre,
    info = AlabasterSlate,
    danger = AlabasterDanger,
    isDark = false,
)

private val DarkPalette = OtteroPalette(
    canvas = ObsidianCanvas,
    recessed = ObsidianSurface,
    card = ObsidianCard,
    ink = ObsidianInk,
    muted = ObsidianMuted,
    border = ObsidianBorder,
    success = ObsidianSage,
    warning = ObsidianSand,
    info = ObsidianSteel,
    danger = ObsidianDanger,
    isDark = true,
)

val LocalOtteroPalette = staticCompositionLocalOf { LightPalette }

private val LightColorScheme = lightColorScheme(
    primary = AlabasterInk,
    onPrimary = AlabasterCard,
    secondary = AlabasterSage,
    onSecondary = AlabasterCard,
    background = AlabasterCanvas,
    onBackground = AlabasterInk,
    surface = AlabasterCard,
    onSurface = AlabasterInk,
    surfaceVariant = AlabasterSurface,
    onSurfaceVariant = AlabasterMuted,
    outline = AlabasterBorder,
    outlineVariant = AlabasterBorder,
    error = AlabasterDanger,
    onError = AlabasterCard,
)

private val DarkColorScheme = darkColorScheme(
    primary = ObsidianInk,
    onPrimary = ObsidianSurface,
    secondary = ObsidianSage,
    onSecondary = ObsidianSurface,
    background = ObsidianCanvas,
    onBackground = ObsidianInk,
    surface = ObsidianCard,
    onSurface = ObsidianInk,
    surfaceVariant = ObsidianSurface,
    onSurfaceVariant = ObsidianMuted,
    outline = ObsidianBorder,
    outlineVariant = ObsidianBorder,
    error = ObsidianDanger,
    onError = ObsidianSurface,
)

@Composable
fun OtteroTheme(
    preference: ThemePreference = ThemePreference.System,
    content: @Composable () -> Unit,
) {
    val dark = when (preference) {
        ThemePreference.System -> isSystemInDarkTheme()
        ThemePreference.Light -> false
        ThemePreference.Dark -> true
    }
    val palette = if (dark) DarkPalette else LightPalette
    val scheme = if (dark) DarkColorScheme else LightColorScheme

    CompositionLocalProvider(LocalOtteroPalette provides palette) {
        MaterialTheme(
            colorScheme = scheme,
            typography = OtteroTypography,
            content = content,
        )
    }
}

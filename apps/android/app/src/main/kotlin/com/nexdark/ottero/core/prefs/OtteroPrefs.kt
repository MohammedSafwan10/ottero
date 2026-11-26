package com.nexdark.ottero.core.prefs

import android.content.Context
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import java.time.DayOfWeek
import java.time.temporal.WeekFields
import java.util.Locale

enum class StartupDestination(val label: String) {
    Today("Today"),
    Notes("Notes"),
    Tasks("Tasks"),
    Habits("Habits"),
    Calendar("Calendar"),
}

enum class WeekStartPreference(val label: String) {
    System("System"),
    Sunday("Sun"),
    Monday("Mon"),
    ;

    fun resolve(): DayOfWeek = when (this) {
        System -> WeekFields.of(Locale.getDefault()).firstDayOfWeek
        Sunday -> DayOfWeek.SUNDAY
        Monday -> DayOfWeek.MONDAY
    }
}

@Immutable
data class OtteroPrefsState(
    val startupDestination: StartupDestination = StartupDestination.Today,
    val weekStart: WeekStartPreference = WeekStartPreference.System,
    val use24HourTime: Boolean = false,
    val showHolidays: Boolean = true,
    val clipboardClearSeconds: Int = 30,
)

val LocalOtteroPrefs = staticCompositionLocalOf { OtteroPrefsState() }

object OtteroPrefsStore {
    private const val PREFS_NAME = "ottero_prefs"
    private const val KEY_STARTUP = "startup_destination"
    private const val KEY_WEEK_START = "week_start"
    private const val KEY_24H = "use_24h_time"
    private const val KEY_HOLIDAYS = "show_holidays"
    private const val KEY_CLIPBOARD_SEC = "clipboard_clear_sec"

    fun load(context: Context): OtteroPrefsState {
        val sp = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val startup = sp.getString(KEY_STARTUP, null)
            ?.let { raw -> StartupDestination.entries.firstOrNull { it.name == raw } }
            ?: StartupDestination.Today
        val weekStart = sp.getString(KEY_WEEK_START, null)
            ?.let { raw -> WeekStartPreference.entries.firstOrNull { it.name == raw } }
            ?: WeekStartPreference.System
        return OtteroPrefsState(
            startupDestination = startup,
            weekStart = weekStart,
            use24HourTime = sp.getBoolean(KEY_24H, false),
            showHolidays = sp.getBoolean(KEY_HOLIDAYS, true),
            clipboardClearSeconds = sp.getInt(KEY_CLIPBOARD_SEC, 30).coerceIn(15, 60),
        )
    }

    fun save(context: Context, prefs: OtteroPrefsState) {
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit()
            .putString(KEY_STARTUP, prefs.startupDestination.name)
            .putString(KEY_WEEK_START, prefs.weekStart.name)
            .putBoolean(KEY_24H, prefs.use24HourTime)
            .putBoolean(KEY_HOLIDAYS, prefs.showHolidays)
            .putInt(KEY_CLIPBOARD_SEC, prefs.clipboardClearSeconds)
            .apply()
    }
}

/** Formats `"HH:MM"` into 24h (`"14:30"`) or 12h (`"2:30 PM"`). */
fun formatDisplayTime(hhmm: String, use24Hour: Boolean): String {
    if (use24Hour) return hhmm
    val parts = hhmm.split(":")
    if (parts.size != 2) return hhmm
    val h = parts[0].toIntOrNull() ?: return hhmm
    val m = parts[1].toIntOrNull() ?: return hhmm
    val suffix = if (h >= 12) "PM" else "AM"
    val h12 = when {
        h == 0 -> 12
        h > 12 -> h - 12
        else -> h
    }
    return String.format(Locale.getDefault(), "%d:%02d %s", h12, m, suffix)
}

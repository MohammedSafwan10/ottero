package com.nexdark.ottero.feature.settings

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.CalendarToday
import androidx.compose.material.icons.outlined.CloudDone
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Palette
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.components.OtteroCardShape
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroSnackbarHost
import com.nexdark.ottero.core.prefs.LocalOtteroPrefs
import com.nexdark.ottero.core.prefs.OtteroPrefsState
import com.nexdark.ottero.core.prefs.StartupDestination
import com.nexdark.ottero.core.prefs.WeekStartPreference
import com.nexdark.ottero.core.security.AccountKeystore
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.AppInfo
import uniffi.ottero_mobile_ffi.MobileAccountStatus
import uniffi.ottero_mobile_ffi.MobileSignInStatus

private enum class SettingsPage(val title: String) {
    Account("Account & Sync"),
    General("General"),
    Appearance("Appearance"),
    Calendar("Calendar"),
    Security("Passwords & Security"),
    About("About"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    onUpdatePrefs: (OtteroPrefsState) -> Unit,
    onBack: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val prefs = LocalOtteroPrefs.current
    val layoutDirection = LocalLayoutDirection.current

    val density = LocalDensity.current
    val windowWidthDp = with(density) { LocalWindowInfo.current.containerSize.width.toDp() }
    val splitPane = windowWidthDp >= 600.dp

    var info by remember { mutableStateOf<AppInfo?>(null) }
    var accountStatus by remember { mutableStateOf<MobileAccountStatus?>(null) }
    val isLive by engine.isRealtimeConnected.collectAsState()
    var activePage by rememberSaveable { mutableStateOf<SettingsPage?>(null) }

    LaunchedEffect(engine) {
        info = runSuspendCatching { engine.appInfo() }.getOrNull()
        engine.changes.onStart { emit(Unit) }.collectLatest {
            accountStatus = runSuspendCatching { engine.accountStatus() }.getOrNull()
        }
    }

    val effectivePage = activePage ?: if (splitPane) SettingsPage.Account else null

    BackHandler(enabled = true) {
        if (!splitPane && activePage != null) {
            activePage = null
        } else {
            onBack()
        }
    }

    Scaffold(
        containerColor = palette.canvas,
        snackbarHost = { OtteroSnackbarHost(feedback) },
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = palette.canvas,
                    titleContentColor = palette.ink,
                    navigationIconContentColor = palette.ink,
                ),
                navigationIcon = {
                    IconButton(
                        onClick = {
                            if (!splitPane && activePage != null) {
                                activePage = null
                            } else {
                                onBack()
                            }
                        },
                    ) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                title = {
                    Text(
                        text = if (!splitPane && activePage != null) {
                            activePage?.title ?: "Settings"
                        } else {
                            "Settings"
                        },
                        style = MaterialTheme.typography.titleMedium,
                    )
                },
            )
        },
    ) { innerPadding ->
        if (splitPane) {
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
            ) {
                SettingsHubList(
                    accountStatus = accountStatus,
                    isLive = isLive,
                    themePreference = themePreference,
                    prefs = prefs,
                    info = info,
                    selectedPage = effectivePage,
                    onSelectPage = { activePage = it },
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                    modifier = Modifier
                        .width(300.dp)
                        .fillMaxHeight(),
                )
                VerticalDivider(color = palette.border)
                SettingsDetailPane(
                    page = effectivePage ?: SettingsPage.Account,
                    engine = engine,
                    accountStatus = accountStatus,
                    isLive = isLive,
                    themePreference = themePreference,
                    prefs = prefs,
                    info = info,
                    onThemeChange = onThemeChange,
                    onUpdatePrefs = onUpdatePrefs,
                    contentPadding = PaddingValues(horizontal = 24.dp, vertical = 12.dp),
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxHeight(),
                )
            }
        } else if (activePage == null) {
            SettingsHubList(
                accountStatus = accountStatus,
                isLive = isLive,
                themePreference = themePreference,
                prefs = prefs,
                info = info,
                selectedPage = null,
                onSelectPage = { activePage = it },
                contentPadding = PaddingValues(
                    start = innerPadding.calculateStartPadding(layoutDirection) + 16.dp,
                    end = innerPadding.calculateEndPadding(layoutDirection) + 16.dp,
                    top = innerPadding.calculateTopPadding() + 8.dp,
                    bottom = innerPadding.calculateBottomPadding() + 32.dp,
                ),
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            SettingsDetailPane(
                page = activePage ?: SettingsPage.Account,
                engine = engine,
                accountStatus = accountStatus,
                isLive = isLive,
                themePreference = themePreference,
                prefs = prefs,
                info = info,
                onThemeChange = onThemeChange,
                onUpdatePrefs = onUpdatePrefs,
                contentPadding = PaddingValues(
                    start = innerPadding.calculateStartPadding(layoutDirection) + 16.dp,
                    end = innerPadding.calculateEndPadding(layoutDirection) + 16.dp,
                    top = innerPadding.calculateTopPadding() + 8.dp,
                    bottom = innerPadding.calculateBottomPadding() + 32.dp,
                ),
                modifier = Modifier
                    .fillMaxSize()
                    .imePadding(),
            )
        }
    }
}

@Composable
private fun SettingsHubList(
    accountStatus: MobileAccountStatus?,
    isLive: Boolean,
    themePreference: ThemePreference,
    prefs: OtteroPrefsState,
    info: AppInfo?,
    selectedPage: SettingsPage?,
    onSelectPage: (SettingsPage) -> Unit,
    contentPadding: PaddingValues,
    modifier: Modifier = Modifier,
) {
    val palette = LocalOtteroPalette.current
    val signedInStatus = accountStatus?.takeIf { it.signedIn }
    val accountSubtitle = if (signedInStatus != null) {
        buildList {
            if (isLive) add("Live")
            signedInStatus.lastSyncedAtMs?.let { add("Synced ${formatRelativeSyncTime(it)}") }
            if (signedInStatus.pendingOutboxCount > 0u) add("${signedInStatus.pendingOutboxCount} pending")
        }.joinToString(" · ").ifEmpty { "Signed in" }
    } else {
        "Sign in to sync across devices"
    }

    LazyColumn(
        modifier = modifier,
        contentPadding = contentPadding,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item(key = "hub-account") {
            SettingsGroupCard {
                SettingsHubRow(
                    icon = if (signedInStatus != null) Icons.Outlined.CloudDone else Icons.Outlined.CloudOff,
                    title = signedInStatus?.email?.takeIf { it.isNotBlank() } ?: "Ottero Account",
                    subtitle = accountSubtitle,
                    trailingValue = null,
                    selected = selectedPage == SettingsPage.Account,
                    onClick = { onSelectPage(SettingsPage.Account) },
                )
            }
        }

        item(key = "hub-preferences") {
            SettingsGroupLabel("Preferences")
            Spacer(Modifier.height(6.dp))
            SettingsGroupCard {
                SettingsHubRow(
                    icon = Icons.Outlined.Tune,
                    title = "General",
                    subtitle = null,
                    trailingValue = prefs.startupDestination.label,
                    selected = selectedPage == SettingsPage.General,
                    onClick = { onSelectPage(SettingsPage.General) },
                )
                HorizontalDivider(
                    color = palette.border,
                    modifier = Modifier.padding(start = 52.dp),
                )
                SettingsHubRow(
                    icon = Icons.Outlined.Palette,
                    title = "Appearance",
                    subtitle = null,
                    trailingValue = themePreference.name,
                    selected = selectedPage == SettingsPage.Appearance,
                    onClick = { onSelectPage(SettingsPage.Appearance) },
                )
            }
        }

        item(key = "hub-modules") {
            SettingsGroupLabel("Modules & Security")
            Spacer(Modifier.height(6.dp))
            SettingsGroupCard {
                val calValue = if (prefs.use24HourTime) "24h" else "12h"
                SettingsHubRow(
                    icon = Icons.Outlined.CalendarToday,
                    title = "Calendar",
                    subtitle = null,
                    trailingValue = calValue,
                    selected = selectedPage == SettingsPage.Calendar,
                    onClick = { onSelectPage(SettingsPage.Calendar) },
                )
                HorizontalDivider(
                    color = palette.border,
                    modifier = Modifier.padding(start = 52.dp),
                )
                val secValue = if (prefs.requireBiometricsForVault) {
                    "Biometrics · ${prefs.clipboardClearSeconds}s"
                } else {
                    "${prefs.clipboardClearSeconds}s"
                }
                SettingsHubRow(
                    icon = Icons.Outlined.Key,
                    title = "Passwords & Security",
                    subtitle = null,
                    trailingValue = secValue,
                    selected = selectedPage == SettingsPage.Security,
                    onClick = { onSelectPage(SettingsPage.Security) },
                )
            }
        }

        item(key = "hub-app") {
            SettingsGroupLabel("App")
            Spacer(Modifier.height(6.dp))
            SettingsGroupCard {
                SettingsHubRow(
                    icon = Icons.Outlined.Info,
                    title = "About",
                    subtitle = null,
                    trailingValue = info?.version?.let { "v$it" },
                    selected = selectedPage == SettingsPage.About,
                    onClick = { onSelectPage(SettingsPage.About) },
                )
            }
        }
    }
}

@Composable
private fun SettingsGroupLabel(text: String) {
    val palette = LocalOtteroPalette.current
    Text(
        text = text,
        style = MaterialTheme.typography.labelMedium,
        color = palette.muted,
        modifier = Modifier.padding(horizontal = 4.dp),
    )
}

@Composable
private fun SettingsGroupCard(
    content: @Composable () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = OtteroCardShape,
        color = palette.card,
        border = androidx.compose.foundation.BorderStroke(1.dp, palette.border),
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            content()
        }
    }
}

@Composable
private fun SettingsHubRow(
    icon: ImageVector,
    title: String,
    subtitle: String?,
    trailingValue: String?,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Surface(
        color = if (selected) palette.recessed else palette.card,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = palette.muted,
                modifier = Modifier.size(20.dp),
            )
            Spacer(Modifier.width(16.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.bodyLarge,
                    color = palette.ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (!subtitle.isNullOrBlank()) {
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = palette.muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (!trailingValue.isNullOrBlank()) {
                Spacer(Modifier.width(8.dp))
                Text(
                    text = trailingValue,
                    style = MaterialTheme.typography.bodySmall,
                    color = palette.muted,
                    maxLines = 1,
                )
            }
            Spacer(Modifier.width(4.dp))
            Icon(
                imageVector = Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                tint = palette.muted,
                modifier = Modifier.size(18.dp),
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun SettingsDetailPane(
    page: SettingsPage,
    engine: OtteroEngine,
    accountStatus: MobileAccountStatus?,
    isLive: Boolean,
    themePreference: ThemePreference,
    prefs: OtteroPrefsState,
    info: AppInfo?,
    onThemeChange: (ThemePreference) -> Unit,
    onUpdatePrefs: (OtteroPrefsState) -> Unit,
    contentPadding: PaddingValues,
    modifier: Modifier = Modifier,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var isSyncing by remember { mutableStateOf(false) }

    LazyColumn(
        modifier = modifier,
        contentPadding = contentPadding,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        when (page) {
            SettingsPage.Account -> {
                item(key = "detail-account") {
                    val status = accountStatus
                    if (status != null && status.signedIn) {
                        OtteroCard {
                            Text(
                                text = "Ottero Account",
                                style = MaterialTheme.typography.titleSmall,
                                color = palette.ink,
                            )
                            Spacer(Modifier.height(6.dp))
                            Text(
                                text = status.email.orEmpty(),
                                style = MaterialTheme.typography.bodyMedium,
                                color = palette.ink,
                            )
                            val syncMeta = buildList {
                                if (isLive) add("Live")
                                status.lastSyncedAtMs?.let { ms ->
                                    add("Last synced ${formatRelativeSyncTime(ms)}")
                                }
                                if (status.pendingOutboxCount > 0u) {
                                    add("${status.pendingOutboxCount} pending")
                                }
                            }.joinToString(" · ")
                            if (syncMeta.isNotEmpty()) {
                                Spacer(Modifier.height(2.dp))
                                Text(
                                    text = syncMeta,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = palette.muted,
                                )
                            }
                            Spacer(Modifier.height(14.dp))
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Button(
                                    enabled = !isSyncing,
                                    onClick = {
                                        isSyncing = true
                                        scope.launch {
                                            runSuspendCatching { engine.syncNow() }
                                                .onSuccess { report ->
                                                    feedback.info("Synced (${report.pushed} sent, ${report.pulled} received)")
                                                }
                                                .onFailure {
                                                    feedback.error(it.message ?: "Sync failed")
                                                }
                                            isSyncing = false
                                        }
                                    },
                                    modifier = Modifier.weight(1f),
                                    shape = RoundedCornerShape(10.dp),
                                    colors = ButtonDefaults.buttonColors(
                                        containerColor = palette.ink,
                                        contentColor = palette.card,
                                    ),
                                ) {
                                    Text(if (isSyncing) "Syncing…" else "Sync now")
                                }
                                OutlinedButton(
                                    enabled = !isSyncing,
                                    onClick = {
                                        feedback.confirmDestructive(
                                            title = "Sign out?",
                                            message = "Passwords will lock until you sign in again. Local notes and tasks stay on this device.",
                                            confirmLabel = "Sign out",
                                        ) {
                                            AccountKeystore.clear(context)
                                            engine.accountSignOut()
                                            feedback.info("Signed out")
                                        }
                                    },
                                    shape = RoundedCornerShape(10.dp),
                                ) {
                                    Text("Sign out", color = palette.danger)
                                }
                            }
                        }
                    } else {
                        AccountSignInCard(
                            engine = engine,
                            onSignedIn = {
                                scope.launch {
                                    runSuspendCatching { engine.syncNow() }
                                        .onSuccess { report ->
                                            feedback.info("Signed in and synced (${report.pulled} received)")
                                        }
                                        .onFailure {
                                            feedback.info("Signed in")
                                        }
                                }
                            },
                        )
                    }
                }
            }

            SettingsPage.General -> {
                item(key = "detail-startup") {
                    OtteroCard {
                        Text(
                            text = "Startup screen",
                            style = MaterialTheme.typography.titleSmall,
                            color = palette.ink,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            text = "Screen shown when opening Ottero.",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.muted,
                        )
                        Spacer(Modifier.height(10.dp))
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            StartupDestination.entries.forEach { dest ->
                                FilterChip(
                                    selected = prefs.startupDestination == dest,
                                    onClick = { onUpdatePrefs(prefs.copy(startupDestination = dest)) },
                                    label = { Text(dest.label) },
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
            }

            SettingsPage.Appearance -> {
                item(key = "detail-theme") {
                    OtteroCard {
                        Text(
                            text = "Theme",
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
            }

            SettingsPage.Calendar -> {
                item(key = "detail-calendar") {
                    OtteroCard {
                        Text(
                            text = "Week starts on",
                            style = MaterialTheme.typography.titleSmall,
                            color = palette.ink,
                        )
                        Spacer(Modifier.height(8.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            WeekStartPreference.entries.forEach { start ->
                                FilterChip(
                                    selected = prefs.weekStart == start,
                                    onClick = { onUpdatePrefs(prefs.copy(weekStart = start)) },
                                    label = { Text(start.label) },
                                    colors = FilterChipDefaults.filterChipColors(
                                        selectedContainerColor = palette.ink,
                                        selectedLabelColor = palette.card,
                                        containerColor = palette.recessed,
                                        labelColor = palette.muted,
                                    ),
                                )
                            }
                        }
                        Spacer(Modifier.height(14.dp))
                        HorizontalDivider(color = palette.border)
                        Spacer(Modifier.height(10.dp))
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = "24-hour time",
                                style = MaterialTheme.typography.bodyMedium,
                                color = palette.ink,
                            )
                            Switch(
                                checked = prefs.use24HourTime,
                                onCheckedChange = { onUpdatePrefs(prefs.copy(use24HourTime = it)) },
                            )
                        }
                        Spacer(Modifier.height(4.dp))
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = "Show holidays",
                                style = MaterialTheme.typography.bodyMedium,
                                color = palette.ink,
                            )
                            Switch(
                                checked = prefs.showHolidays,
                                onCheckedChange = { onUpdatePrefs(prefs.copy(showHolidays = it)) },
                            )
                        }
                    }
                }
            }

            SettingsPage.Security -> {
                item(key = "detail-security") {
                    OtteroCard {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = "Biometric unlock",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = palette.ink,
                                )
                                Text(
                                    text = "Require fingerprint, face, or device PIN for Passwords.",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = palette.muted,
                                )
                            }
                            Spacer(Modifier.width(12.dp))
                            Switch(
                                checked = prefs.requireBiometricsForVault,
                                onCheckedChange = {
                                    onUpdatePrefs(prefs.copy(requireBiometricsForVault = it))
                                },
                            )
                        }
                        Spacer(Modifier.height(14.dp))
                        HorizontalDivider(color = palette.border)
                        Spacer(Modifier.height(12.dp))
                        Text(
                            text = "Clipboard auto-clear",
                            style = MaterialTheme.typography.titleSmall,
                            color = palette.ink,
                        )
                        Spacer(Modifier.height(8.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            listOf(15, 30, 60).forEach { seconds ->
                                FilterChip(
                                    selected = prefs.clipboardClearSeconds == seconds,
                                    onClick = { onUpdatePrefs(prefs.copy(clipboardClearSeconds = seconds)) },
                                    label = { Text("${seconds}s") },
                                    colors = FilterChipDefaults.filterChipColors(
                                        selectedContainerColor = palette.ink,
                                        selectedLabelColor = palette.card,
                                        containerColor = palette.recessed,
                                        labelColor = palette.muted,
                                    ),
                                )
                            }
                        }
                        Spacer(Modifier.height(14.dp))
                        HorizontalDivider(color = palette.border)
                        Spacer(Modifier.height(12.dp))
                        OutlinedButton(
                            onClick = {
                                val intent = Intent(Settings.ACTION_REQUEST_SET_AUTOFILL_SERVICE).apply {
                                    data = Uri.parse("package:${context.packageName}")
                                }
                                runCatching { context.startActivity(intent) }
                                    .onFailure {
                                        runCatching {
                                            context.startActivity(Intent(Settings.ACTION_SETTINGS))
                                        }
                                    }
                            },
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(10.dp),
                        ) {
                            Text("Configure Android Autofill", color = palette.ink)
                        }
                    }
                }
            }

            SettingsPage.About -> {
                item(key = "detail-about") {
                    OtteroCard {
                        val loadedInfo = info
                        Text(
                            text = if (loadedInfo != null) "Ottero ${loadedInfo.version}" else "Ottero",
                            style = MaterialTheme.typography.titleSmall,
                            color = palette.ink,
                        )
                        Spacer(Modifier.height(4.dp))
                        Text(
                            text = "Local-first personal workspace with end-to-end encrypted sync.",
                            style = MaterialTheme.typography.bodySmall,
                            color = palette.muted,
                        )
                        if (loadedInfo != null) {
                            Spacer(Modifier.height(8.dp))
                            Text(
                                text = "Database schema v${loadedInfo.schemaVersion}",
                                style = MaterialTheme.typography.bodySmall,
                                color = palette.muted,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AccountSignInCard(
    engine: OtteroEngine,
    onSignedIn: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var secretKey by remember { mutableStateOf("") }
    var showPassword by remember { mutableStateOf(false) }
    var needsTwoStep by remember { mutableStateOf(false) }
    var twoStepCode by remember { mutableStateOf("") }
    var useBackupCode by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }

    DisposableEffect(Unit) {
        onDispose {
            password = ""
            secretKey = ""
            twoStepCode = ""
        }
    }

    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedContainerColor = palette.canvas,
        unfocusedContainerColor = palette.canvas,
        focusedBorderColor = palette.ink,
        unfocusedBorderColor = palette.border,
        focusedTextColor = palette.ink,
        unfocusedTextColor = palette.ink,
    )

    OtteroCard {
        Text(
            text = if (needsTwoStep) "Two-step verification" else "Sign in to Ottero Cloud",
            style = MaterialTheme.typography.titleSmall,
            color = palette.ink,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            text = if (needsTwoStep) {
                if (useBackupCode) "Enter one of your backup codes." else "Enter the 6-digit code from your authenticator app."
            } else {
                "Sign in to sync Notes, Tasks, Calendar, Habits, and Passwords across devices."
            },
            style = MaterialTheme.typography.bodySmall,
            color = palette.muted,
        )
        Spacer(Modifier.height(12.dp))

        if (!needsTwoStep) {
            OutlinedTextField(
                value = email,
                onValueChange = { email = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Email") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Email,
                    autoCorrectEnabled = false,
                    imeAction = ImeAction.Next,
                ),
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )
            Spacer(Modifier.height(10.dp))
            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Password") },
                singleLine = true,
                visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Password,
                    autoCorrectEnabled = false,
                    imeAction = ImeAction.Next,
                ),
                trailingIcon = {
                    IconButton(onClick = { showPassword = !showPassword }) {
                        Icon(
                            imageVector = if (showPassword) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                            contentDescription = if (showPassword) "Hide password" else "Show password",
                            tint = palette.muted,
                        )
                    }
                },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )
            Spacer(Modifier.height(10.dp))
            OutlinedTextField(
                value = secretKey,
                onValueChange = { secretKey = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Secret Key (A3-...)") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Password,
                    autoCorrectEnabled = false,
                    imeAction = ImeAction.Done,
                ),
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )
            Spacer(Modifier.height(14.dp))
            Button(
                enabled = !busy && email.isNotBlank() && password.isNotBlank() && secretKey.isNotBlank(),
                onClick = {
                    busy = true
                    scope.launch {
                        runSuspendCatching { engine.accountSignIn(email, password, secretKey) }
                            .onSuccess { res ->
                                password = ""
                                secretKey = ""
                                when (res.status) {
                                    MobileSignInStatus.NEEDS_TWO_STEP -> needsTwoStep = true
                                    MobileSignInStatus.SIGNED_IN -> {
                                        res.session?.let { AccountKeystore.save(context, it) }
                                        onSignedIn()
                                    }
                                }
                            }
                            .onFailure {
                                feedback.error(it.message ?: "Sign-in failed")
                            }
                        busy = false
                    }
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(if (busy) "Signing in…" else "Sign in")
            }
        } else {
            OutlinedTextField(
                value = twoStepCode,
                onValueChange = { twoStepCode = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text(if (useBackupCode) "Backup code" else "6-digit code") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (useBackupCode) KeyboardType.Text else KeyboardType.Number,
                    autoCorrectEnabled = false,
                    imeAction = ImeAction.Done,
                ),
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )
            Spacer(Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                TextButton(
                    enabled = !busy,
                    onClick = {
                        useBackupCode = !useBackupCode
                        twoStepCode = ""
                    },
                ) {
                    Text(if (useBackupCode) "Use authenticator code" else "Use backup code", color = palette.muted)
                }
                TextButton(
                    enabled = !busy,
                    onClick = {
                        needsTwoStep = false
                        twoStepCode = ""
                    },
                ) {
                    Text("Cancel", color = palette.muted)
                }
            }
            Spacer(Modifier.height(6.dp))
            Button(
                enabled = !busy && twoStepCode.isNotBlank(),
                onClick = {
                    busy = true
                    scope.launch {
                        runSuspendCatching { engine.accountVerifyTwoStep(twoStepCode, useBackupCode) }
                            .onSuccess { res ->
                                twoStepCode = ""
                                needsTwoStep = false
                                res.session?.let { AccountKeystore.save(context, it) }
                                onSignedIn()
                            }
                            .onFailure {
                                feedback.error(it.message ?: "Verification failed")
                            }
                        busy = false
                    }
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(if (busy) "Verifying…" else "Verify")
            }
        }
    }
}

private fun formatRelativeSyncTime(epochMs: Long): String {
    val diffSec = ((System.currentTimeMillis() - epochMs) / 1000L).coerceAtLeast(0L)
    return when {
        diffSec < 60L -> "just now"
        diffSec < 3600L -> "${diffSec / 60L}m ago"
        diffSec < 86400L -> "${diffSec / 3600L}h ago"
        else -> "${diffSec / 86400L}d ago"
    }
}

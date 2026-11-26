package com.nexdark.ottero.feature.settings

import androidx.activity.compose.BackHandler
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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.feedback.OtteroSnackbarHost
import com.nexdark.ottero.core.security.AccountKeystore
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import com.nexdark.ottero.ui.theme.ThemePreference
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.AppInfo
import uniffi.ottero_mobile_ffi.MobileAccountStatus
import uniffi.ottero_mobile_ffi.MobileSignInStatus

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    engine: OtteroEngine,
    themePreference: ThemePreference,
    onThemeChange: (ThemePreference) -> Unit,
    onBack: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val context = LocalContext.current
    val layoutDirection = LocalLayoutDirection.current
    val scope = rememberCoroutineScope()

    var info by remember { mutableStateOf<AppInfo?>(null) }
    var accountStatus by remember { mutableStateOf<MobileAccountStatus?>(null) }
    var isSyncing by remember { mutableStateOf(false) }

    BackHandler(enabled = true) { onBack() }

    LaunchedEffect(engine) {
        info = runSuspendCatching { engine.appInfo() }.getOrNull()
        engine.changes.onStart { emit(Unit) }.collectLatest {
            accountStatus = runSuspendCatching { engine.accountStatus() }.getOrNull()
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
                    IconButton(onClick = onBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Outlined.ArrowBack,
                            contentDescription = "Back",
                        )
                    }
                },
                title = {
                    Text(
                        text = "Settings",
                        style = MaterialTheme.typography.titleMedium,
                    )
                },
            )
        },
    ) { innerPadding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .imePadding(),
            contentPadding = PaddingValues(
                start = innerPadding.calculateStartPadding(layoutDirection) + 16.dp,
                end = innerPadding.calculateEndPadding(layoutDirection) + 16.dp,
                top = innerPadding.calculateTopPadding() + 8.dp,
                bottom = innerPadding.calculateBottomPadding() + 32.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            item(key = "account-card", contentType = "account") {
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
                        if (status.pendingOutboxCount > 0u) {
                            Spacer(Modifier.height(2.dp))
                            Text(
                                text = "${status.pendingOutboxCount} pending changes",
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
                    Text(
                        text = if (loadedInfo != null) "Ottero ${loadedInfo.version}" else "Ottero",
                        style = MaterialTheme.typography.bodyMedium,
                        color = palette.ink,
                    )
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
            text = "Ottero Account",
            style = MaterialTheme.typography.titleSmall,
            color = palette.ink,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            text = "Sign in to sync across devices and unlock Passwords.",
            style = MaterialTheme.typography.bodySmall,
            color = palette.muted,
        )
        Spacer(Modifier.height(14.dp))

        if (!needsTwoStep) {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = email,
                    onValueChange = { email = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = { Text("Email", color = palette.muted) },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Email,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Next,
                    ),
                    shape = RoundedCornerShape(12.dp),
                    colors = fieldColors,
                )
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = { Text("Password", color = palette.muted) },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Password,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Next,
                    ),
                    visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
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
                OutlinedTextField(
                    value = secretKey,
                    onValueChange = { secretKey = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = { Text("Secret Key (from Emergency Kit)", color = palette.muted) },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Password,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Done,
                    ),
                    shape = RoundedCornerShape(12.dp),
                    colors = fieldColors,
                )
                Spacer(Modifier.height(2.dp))
                Button(
                    enabled = !busy && email.isNotBlank() && password.isNotEmpty() && secretKey.isNotBlank(),
                    onClick = {
                        val e = email.trim()
                        val p = password
                        val sk = secretKey.trim()
                        busy = true
                        scope.launch {
                            runSuspendCatching { engine.accountSignIn(e, p, sk) }
                                .onSuccess { res ->
                                    password = ""
                                    secretKey = ""
                                    if (res.status == MobileSignInStatus.NEEDS_TWO_STEP) {
                                        needsTwoStep = true
                                    } else {
                                        res.session?.let { AccountKeystore.save(context, it) }
                                        onSignedIn()
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
            }
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = twoStepCode,
                    onValueChange = { twoStepCode = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    placeholder = {
                        Text(
                            if (useBackupCode) "Backup code" else "6-digit authenticator code",
                            color = palette.muted,
                        )
                    },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = if (useBackupCode) KeyboardType.Text else KeyboardType.Number,
                        autoCorrectEnabled = false,
                        imeAction = ImeAction.Done,
                    ),
                    shape = RoundedCornerShape(12.dp),
                    colors = fieldColors,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    TextButton(onClick = { useBackupCode = !useBackupCode }) {
                        Text(
                            if (useBackupCode) "Use authenticator code" else "Use backup code",
                            color = palette.muted,
                        )
                    }
                    TextButton(onClick = {
                        needsTwoStep = false
                        twoStepCode = ""
                    }) {
                        Text("Back", color = palette.muted)
                    }
                }
                Button(
                    enabled = !busy && twoStepCode.isNotBlank(),
                    onClick = {
                        val c = twoStepCode.trim()
                        busy = true
                        scope.launch {
                            runSuspendCatching { engine.accountVerifyTwoStep(c, useBackupCode) }
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
}

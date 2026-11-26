package com.nexdark.ottero.feature.passwords

import android.app.Activity
import android.view.WindowManager
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Fingerprint
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.fragment.app.FragmentActivity
import com.nexdark.ottero.core.components.OtteroPullToRefresh
import com.nexdark.ottero.core.prefs.LocalOtteroPrefs
import com.nexdark.ottero.core.security.BiometricHelper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.EmptyPlaceholder
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.onStart
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.MobileVaultStatus
import uniffi.ottero_mobile_ffi.VaultEntryDetail
import uniffi.ottero_mobile_ffi.VaultEntrySummary

@OptIn(FlowPreview::class)
@Composable
fun PasswordsScreen(
    engine: OtteroEngine,
    contentPadding: PaddingValues,
    onOpenSettings: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val prefs = LocalOtteroPrefs.current
    val layoutDirection = LocalLayoutDirection.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var status by remember { mutableStateOf<MobileVaultStatus?>(null) }
    var query by remember { mutableStateOf("") }
    var favoritesOnly by remember { mutableStateOf(false) }
    var biometricVerified by remember { mutableStateOf(!prefs.requireBiometricsForVault) }
    var items by remember { mutableStateOf<List<VaultEntrySummary>>(emptyList()) }
    var isLoading by remember { mutableStateOf(true) }
    var editingDetail by remember { mutableStateOf<VaultEntryDetail?>(null) }
    var showNewSheet by remember { mutableStateOf(false) }

    val activity = context as? FragmentActivity
    val needsBiometric = prefs.requireBiometricsForVault && !biometricVerified &&
        activity != null && BiometricHelper.canAuthenticate(activity)

    LaunchedEffect(needsBiometric, status) {
        if (needsBiometric && status == MobileVaultStatus.UNLOCKED) {
            BiometricHelper.authenticate(
                activity = activity,
                onSuccess = { biometricVerified = true },
                onError = { feedback.error(it) },
            )
        }
    }

    // Block screenshots & Recents previews while viewing Passwords.
    DisposableEffect(context) {
        val window = (context as? Activity)?.window
        window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose {
            window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }

    LaunchedEffect(engine) {
        combine(
            snapshotFlow { query }.debounce(150),
            engine.changes.onStart { emit(Unit) },
        ) { q, _ -> q }.collectLatest { q ->
            runSuspendCatching {
                val s = engine.vaultStatus()
                val list = if (s == MobileVaultStatus.UNLOCKED) {
                    engine.vaultListItems(q.ifBlank { null })
                } else {
                    emptyList()
                }
                s to list
            }.onSuccess { (s, list) ->
                status = s
                items = list
                if (s != MobileVaultStatus.UNLOCKED) {
                    query = ""
                    editingDetail = null
                    showNewSheet = false
                }
                isLoading = false
            }.onFailure {
                isLoading = false
                feedback.error(it.message ?: "Could not load vault")
            }
        }
    }

    if (showNewSheet || editingDetail != null) {
        VaultItemSheet(
            engine = engine,
            existing = editingDetail,
            onDismiss = {
                showNewSheet = false
                editingDetail = null
            },
        )
    }

    val outerPadding = PaddingValues(
        start = contentPadding.calculateStartPadding(layoutDirection) + 16.dp,
        end = contentPadding.calculateEndPadding(layoutDirection) + 16.dp,
        top = contentPadding.calculateTopPadding() + 8.dp,
        bottom = contentPadding.calculateBottomPadding() + 88.dp,
    )

    when (status) {
        null -> Unit

        MobileVaultStatus.NOT_SET_UP -> {
            VaultAccountPromptPane(
                contentPadding = outerPadding,
                onOpenSettings = onOpenSettings,
            )
        }

        MobileVaultStatus.LOCKED -> {
            VaultUnlockPane(
                contentPadding = outerPadding,
                onUnlock = { pw ->
                    runSuspendCatching { engine.vaultUnlock(pw) }
                        .onFailure { feedback.error(it.message ?: "Incorrect master password") }
                },
            )
        }

        MobileVaultStatus.UNLOCKED -> {
            if (needsBiometric) {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(outerPadding),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    OtteroCard {
                        EmptyPlaceholder(
                            icon = Icons.Outlined.Fingerprint,
                            title = "Passwords locked",
                            hint = "Verify with biometrics to view your vault.",
                            actionLabel = "Unlock",
                            onAction = {
                                BiometricHelper.authenticate(
                                    activity = activity,
                                    onSuccess = { biometricVerified = true },
                                    onError = { feedback.error(it) },
                                )
                            },
                        )
                    }
                }
            } else {
                val displayed = remember(items, favoritesOnly) {
                    if (favoritesOnly) items.filter { it.favorite } else items
                }
                OtteroPullToRefresh(engine = engine, modifier = Modifier.fillMaxSize()) {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxSize()
                            .imePadding(),
                        contentPadding = outerPadding,
                        verticalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        item(key = "vault-controls", contentType = "controls") {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                OutlinedTextField(
                                    value = query,
                                    onValueChange = { query = it },
                                    modifier = Modifier.weight(1f),
                                    singleLine = true,
                                    placeholder = { Text("Search vault", color = palette.muted) },
                                    leadingIcon = {
                                        Icon(
                                            imageVector = Icons.Outlined.Search,
                                            contentDescription = null,
                                            tint = palette.muted,
                                        )
                                    },
                                    trailingIcon = if (query.isNotEmpty()) {
                                        {
                                            IconButton(onClick = { query = "" }) {
                                                Icon(
                                                    imageVector = Icons.Outlined.Close,
                                                    contentDescription = "Clear search",
                                                    tint = palette.muted,
                                                )
                                            }
                                        }
                                    } else {
                                        null
                                    },
                                    shape = RoundedCornerShape(12.dp),
                                    colors = OutlinedTextFieldDefaults.colors(
                                        focusedContainerColor = palette.card,
                                        unfocusedContainerColor = palette.card,
                                        focusedBorderColor = palette.ink,
                                        unfocusedBorderColor = palette.border,
                                        focusedTextColor = palette.ink,
                                        unfocusedTextColor = palette.ink,
                                    ),
                                )
                                IconButton(
                                    onClick = {
                                        scope.launch {
                                            runSuspendCatching { engine.vaultLock() }
                                                .onSuccess { feedback.info("Vault locked") }
                                        }
                                    },
                                    modifier = Modifier.size(48.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Outlined.Lock,
                                        contentDescription = "Lock vault",
                                        tint = palette.muted,
                                    )
                                }
                            }
                        }

                        item(key = "vault-filters", contentType = "filters") {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                FilterChip(
                                    selected = !favoritesOnly,
                                    onClick = { favoritesOnly = false },
                                    label = { Text("All") },
                                    colors = FilterChipDefaults.filterChipColors(
                                        selectedContainerColor = palette.ink,
                                        selectedLabelColor = palette.card,
                                        containerColor = palette.card,
                                        labelColor = palette.muted,
                                    ),
                                )
                                FilterChip(
                                    selected = favoritesOnly,
                                    onClick = { favoritesOnly = true },
                                    label = { Text("Favorites") },
                                    colors = FilterChipDefaults.filterChipColors(
                                        selectedContainerColor = palette.ink,
                                        selectedLabelColor = palette.card,
                                        containerColor = palette.card,
                                        labelColor = palette.muted,
                                    ),
                                )
                                Spacer(Modifier.weight(1f))
                                OutlinedButton(
                                    onClick = { showNewSheet = true },
                                    shape = RoundedCornerShape(10.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Outlined.Add,
                                        contentDescription = null,
                                        tint = palette.ink,
                                        modifier = Modifier.size(16.dp),
                                    )
                                    Spacer(Modifier.width(4.dp))
                                    Text("New", color = palette.ink)
                                }
                            }
                        }

                        if (!isLoading && displayed.isEmpty()) {
                            item(key = "vault-empty", contentType = "empty") {
                                OtteroCard {
                                    EmptyPlaceholder(
                                        icon = Icons.Outlined.Key,
                                        title = if (query.isBlank() && !favoritesOnly) "Vault is empty" else "No matches",
                                        hint = if (query.isBlank() && !favoritesOnly) "Add a login, card, or secure note." else "Try another filter.",
                                    )
                                }
                            }
                        } else {
                            items(
                                items = displayed,
                                key = { "vault-${it.id}" },
                                contentType = { "vault_card" },
                            ) { item ->
                                OtteroCard(
                                    modifier = Modifier.clickable {
                                        scope.launch {
                                            runSuspendCatching { engine.vaultGetItem(item.id) }
                                                .onSuccess { editingDetail = it }
                                                .onFailure { feedback.error(it.message ?: "Could not open item") }
                                        }
                                    },
                                ) {
                                    Row(
                                        modifier = Modifier.fillMaxWidth(),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Column(modifier = Modifier.weight(1f)) {
                                            Text(
                                                text = item.title,
                                                style = MaterialTheme.typography.titleSmall,
                                                color = palette.ink,
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                            val sub = buildList {
                                                if (item.subtitle.isNotBlank()) add(item.subtitle)
                                                if (item.kind != "login") add(item.kind.replaceFirstChar { it.uppercase() })
                                            }.joinToString(" · ")
                                            if (sub.isNotBlank()) {
                                                Spacer(Modifier.height(2.dp))
                                                Text(
                                                    text = sub,
                                                    style = MaterialTheme.typography.bodySmall,
                                                    color = palette.muted,
                                                    maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis,
                                                )
                                            }
                                        }
                                        IconButton(
                                            onClick = {
                                                scope.launch {
                                                    runSuspendCatching { engine.vaultToggleFavorite(item.id) }
                                                        .onFailure { feedback.error(it.message ?: "Could not update favorite") }
                                                }
                                            },
                                            modifier = Modifier.size(40.dp),
                                        ) {
                                            Icon(
                                                imageVector = if (item.favorite) Icons.Filled.Star else Icons.Outlined.StarBorder,
                                                contentDescription = if (item.favorite) "Remove favorite" else "Favorite",
                                                tint = if (item.favorite) palette.ink else palette.muted,
                                                modifier = Modifier.size(20.dp),
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun VaultAccountPromptPane(
    contentPadding: PaddingValues,
    onOpenSettings: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(contentPadding),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        OtteroCard {
            Text(
                text = "Sign in to use Passwords",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = "Your Ottero account unlocks Passwords and keeps them synced across your devices.",
                style = MaterialTheme.typography.bodySmall,
                color = palette.muted,
            )
            Spacer(Modifier.height(16.dp))
            Button(
                onClick = onOpenSettings,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text("Open Settings")
            }
        }
    }
}

@Composable
private fun VaultUnlockPane(
    contentPadding: PaddingValues,
    onUnlock: suspend (String) -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val scope = rememberCoroutineScope()
    var password by remember { mutableStateOf("") }
    var show by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }

    DisposableEffect(Unit) {
        onDispose { password = "" }
    }

    fun submit() {
        if (busy || password.isEmpty()) return
        val submitted = password
        busy = true
        scope.launch {
            onUnlock(submitted)
            password = ""
            busy = false
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .imePadding()
            .padding(contentPadding),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        OtteroCard {
            Text(
                text = "Vault locked",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )
            Spacer(Modifier.height(14.dp))
            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Master password", color = palette.muted) },
                visualTransformation = if (show) VisualTransformation.None else PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Password,
                    autoCorrectEnabled = false,
                    imeAction = ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = { submit() }),
                trailingIcon = {
                    IconButton(onClick = { show = !show }) {
                        Icon(
                            imageVector = if (show) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                            contentDescription = if (show) "Hide" else "Show",
                            tint = palette.muted,
                        )
                    }
                },
                shape = RoundedCornerShape(12.dp),
            )
            Spacer(Modifier.height(14.dp))
            Button(
                enabled = !busy && password.isNotEmpty(),
                onClick = ::submit,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp),
                shape = RoundedCornerShape(12.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = palette.ink,
                    contentColor = palette.card,
                ),
            ) {
                Text(if (busy) "Unlocking…" else "Unlock")
            }
        }
    }
}

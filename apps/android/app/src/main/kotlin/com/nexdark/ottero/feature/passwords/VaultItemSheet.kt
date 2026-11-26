package com.nexdark.ottero.feature.passwords

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Casino
import androidx.compose.material.icons.outlined.ContentCopy
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
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.SecureFlagPolicy
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.components.OtteroCard
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.prefs.LocalOtteroPrefs
import com.nexdark.ottero.core.security.SensitiveClipboard
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.TotpCodeSnapshot
import uniffi.ottero_mobile_ffi.VaultEntryDetail

private val VaultKinds = listOf("login" to "Login", "card" to "Card", "note" to "Secure note")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VaultItemSheet(
    engine: OtteroEngine,
    existing: VaultEntryDetail?,
    onDismiss: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val prefs = LocalOtteroPrefs.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    val key = existing?.id ?: "new-vault-item"
    var kind by remember(key) { mutableStateOf(existing?.kind ?: "login") }
    var title by remember(key) { mutableStateOf(existing?.title.orEmpty()) }
    var username by remember(key) { mutableStateOf(existing?.username.orEmpty()) }
    var password by remember(key) { mutableStateOf(existing?.password.orEmpty()) }
    var url by remember(key) { mutableStateOf(existing?.url.orEmpty()) }
    var totpSecret by remember(key) { mutableStateOf(existing?.totpSecret.orEmpty()) }
    var cardholder by remember(key) { mutableStateOf(existing?.cardholder.orEmpty()) }
    var cardNumber by remember(key) { mutableStateOf(existing?.cardNumber.orEmpty()) }
    var cardExpiry by remember(key) { mutableStateOf(existing?.cardExpiry.orEmpty()) }
    var cardCvv by remember(key) { mutableStateOf(existing?.cardCvv.orEmpty()) }
    var notes by remember(key) { mutableStateOf(existing?.notes.orEmpty()) }
    var showSecret by remember(key) { mutableStateOf(false) }
    var totpSnapshot by remember(key) { mutableStateOf<TotpCodeSnapshot?>(null) }
    var busy by remember { mutableStateOf(false) }

    LaunchedEffect(kind, totpSecret) {
        if (kind != "login" || totpSecret.isBlank()) {
            totpSnapshot = null
            return@LaunchedEffect
        }
        while (true) {
            totpSnapshot = runSuspendCatching { engine.vaultTotpNow(totpSecret) }.getOrNull()
            delay(1_000L)
        }
    }

    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedContainerColor = palette.card,
        unfocusedContainerColor = palette.card,
        focusedBorderColor = palette.ink,
        unfocusedBorderColor = palette.border,
        focusedTextColor = palette.ink,
        unfocusedTextColor = palette.ink,
    )

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = palette.canvas,
        properties = ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = if (existing == null) "New vault item" else "Edit ${VaultKinds.firstOrNull { it.first == kind }?.second?.lowercase() ?: "item"}",
                style = MaterialTheme.typography.titleMedium,
                color = palette.ink,
            )

            if (existing == null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    VaultKinds.forEach { (code, label) ->
                        FilterChip(
                            selected = kind == code,
                            onClick = { kind = code },
                            label = { Text(label) },
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

            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Title") },
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            when (kind) {
                "login" -> {
                    OutlinedTextField(
                        value = username,
                        onValueChange = { username = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Username or email") },
                        singleLine = true,
                        trailingIcon = if (username.isNotBlank()) {
                            {
                                IconButton(
                                    onClick = {
                                        SensitiveClipboard.copySecret(context, "Username", username, prefs.clipboardClearSeconds)
                                        feedback.info("Username copied")
                                    },
                                ) {
                                    Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy username", modifier = Modifier.size(18.dp))
                                }
                            }
                        } else null,
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )

                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Password") },
                        singleLine = true,
                        visualTransformation = if (showSecret) VisualTransformation.None else PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(
                            keyboardType = KeyboardType.Password,
                            autoCorrectEnabled = false,
                        ),
                        trailingIcon = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                IconButton(
                                    onClick = {
                                        scope.launch {
                                            runSuspendCatching { engine.vaultGeneratePassword(20) }
                                                .onSuccess { password = it; showSecret = true }
                                        }
                                    },
                                ) {
                                    Icon(Icons.Outlined.Casino, contentDescription = "Generate password", modifier = Modifier.size(18.dp))
                                }
                                IconButton(onClick = { showSecret = !showSecret }) {
                                    Icon(
                                        imageVector = if (showSecret) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                        contentDescription = if (showSecret) "Hide password" else "Show password",
                                        modifier = Modifier.size(18.dp),
                                    )
                                }
                                if (password.isNotBlank()) {
                                    IconButton(
                                        onClick = {
                                            SensitiveClipboard.copySecret(context, "Password", password, prefs.clipboardClearSeconds)
                                            feedback.info("Password copied (${prefs.clipboardClearSeconds}s auto-clear)")
                                        },
                                    ) {
                                        Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy password", modifier = Modifier.size(18.dp))
                                    }
                                }
                            }
                        },
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )

                    OutlinedTextField(
                        value = url,
                        onValueChange = { url = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Website") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )

                    OutlinedTextField(
                        value = totpSecret,
                        onValueChange = { totpSecret = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Authenticator key (TOTP)") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )

                    val snap = totpSnapshot
                    if (snap != null) {
                        OtteroCard {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column {
                                    Text(
                                        text = snap.grouped,
                                        style = MaterialTheme.typography.titleLarge.copy(
                                            fontWeight = FontWeight.SemiBold,
                                            fontFeatureSettings = "tnum",
                                        ),
                                        color = palette.ink,
                                    )
                                    Text(
                                        text = "${snap.secondsLeft}s remaining",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = palette.muted,
                                    )
                                }
                                IconButton(
                                    onClick = {
                                        SensitiveClipboard.copySecret(context, "Verification code", snap.code, prefs.clipboardClearSeconds)
                                        feedback.info("Code copied")
                                    },
                                ) {
                                    Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy code")
                                }
                            }
                        }
                    }
                }

                "card" -> {
                    OutlinedTextField(
                        value = cardholder,
                        onValueChange = { cardholder = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Cardholder name") },
                        singleLine = true,
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )
                    OutlinedTextField(
                        value = cardNumber,
                        onValueChange = { cardNumber = it },
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("Card number") },
                        singleLine = true,
                        visualTransformation = if (showSecret) VisualTransformation.None else PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
                        trailingIcon = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                IconButton(onClick = { showSecret = !showSecret }) {
                                    Icon(
                                        imageVector = if (showSecret) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                        contentDescription = "Toggle visibility",
                                        modifier = Modifier.size(18.dp),
                                    )
                                }
                                if (cardNumber.isNotBlank()) {
                                    IconButton(
                                        onClick = {
                                            SensitiveClipboard.copySecret(context, "Card number", cardNumber, prefs.clipboardClearSeconds)
                                            feedback.info("Card number copied")
                                        },
                                    ) {
                                        Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy card number", modifier = Modifier.size(18.dp))
                                    }
                                }
                            }
                        },
                        shape = RoundedCornerShape(12.dp),
                        colors = fieldColors,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        OutlinedTextField(
                            value = cardExpiry,
                            onValueChange = { cardExpiry = it },
                            modifier = Modifier.weight(1f),
                            label = { Text("Expiry (MM/YY)") },
                            singleLine = true,
                            shape = RoundedCornerShape(12.dp),
                            colors = fieldColors,
                        )
                        OutlinedTextField(
                            value = cardCvv,
                            onValueChange = { cardCvv = it },
                            modifier = Modifier.weight(1f),
                            label = { Text("CVV") },
                            singleLine = true,
                            visualTransformation = if (showSecret) VisualTransformation.None else PasswordVisualTransformation(),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
                            shape = RoundedCornerShape(12.dp),
                            colors = fieldColors,
                        )
                    }
                }
            }

            OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text(if (kind == "note") "Secure note" else "Notes") },
                minLines = if (kind == "note") 4 else 2,
                maxLines = if (kind == "note") 10 else 4,
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            Spacer(Modifier.height(4.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (existing != null) {
                    OutlinedButton(
                        enabled = !busy,
                        onClick = {
                            busy = true
                            scope.launch {
                                runSuspendCatching { engine.vaultTrashItem(existing.id) }
                                    .onSuccess {
                                        onDismiss()
                                        feedback.undo("Moved to trash") { engine.vaultRestoreItem(existing.id) }
                                    }
                                    .onFailure {
                                        busy = false
                                        feedback.error(it.message ?: "Could not delete item")
                                    }
                            }
                        },
                        shape = RoundedCornerShape(10.dp),
                    ) {
                        Text("Delete", color = palette.danger)
                    }
                }

                Button(
                    enabled = !busy && title.isNotBlank(),
                    onClick = {
                        busy = true
                        scope.launch {
                            runSuspendCatching {
                                when (kind) {
                                    "card" -> engine.vaultSaveCard(
                                        id = existing?.id,
                                        title = title.trim(),
                                        cardholder = cardholder.trim(),
                                        number = cardNumber.trim(),
                                        expiry = cardExpiry.trim(),
                                        cvv = cardCvv.trim(),
                                        notes = notes,
                                    )
                                    "note" -> engine.vaultSaveSecureNote(
                                        id = existing?.id,
                                        title = title.trim(),
                                        notes = notes,
                                    )
                                    else -> engine.vaultSaveLogin(
                                        id = existing?.id,
                                        title = title.trim(),
                                        username = username.trim(),
                                        password = password,
                                        url = url.trim(),
                                        totpSecret = totpSecret.trim(),
                                        notes = notes,
                                    )
                                }
                            }.onSuccess {
                                onDismiss()
                            }.onFailure {
                                busy = false
                                feedback.error(it.message ?: "Could not save item")
                            }
                        }
                    },
                    modifier = Modifier.weight(1f),
                    shape = RoundedCornerShape(10.dp),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = palette.ink,
                        contentColor = palette.card,
                    ),
                ) {
                    Text("Save")
                }
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}

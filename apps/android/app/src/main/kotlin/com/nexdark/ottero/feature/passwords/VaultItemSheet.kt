package com.nexdark.ottero.feature.passwords

import androidx.compose.foundation.BorderStroke
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ModalBottomSheetProperties
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
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
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.SecureFlagPolicy
import com.nexdark.ottero.core.async.runSuspendCatching
import com.nexdark.ottero.core.bridge.OtteroEngine
import com.nexdark.ottero.core.feedback.LocalOtteroFeedback
import com.nexdark.ottero.core.security.SensitiveClipboard
import com.nexdark.ottero.ui.theme.LocalOtteroPalette
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.ottero_mobile_ffi.TotpCodeSnapshot
import uniffi.ottero_mobile_ffi.VaultEntryDetail

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VaultItemSheet(
    engine: OtteroEngine,
    existing: VaultEntryDetail?,
    onDismiss: () -> Unit,
) {
    val palette = LocalOtteroPalette.current
    val feedback = LocalOtteroFeedback.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)

    // Keep all secret fields in RAM only (`remember`, never `rememberSaveable` Bundle).
    val key = existing?.id ?: "new"
    var title by remember(key) { mutableStateOf(existing?.title.orEmpty()) }
    var username by remember(key) { mutableStateOf(existing?.username.orEmpty()) }
    var password by remember(key) { mutableStateOf(existing?.password.orEmpty()) }
    var url by remember(key) { mutableStateOf(existing?.url.orEmpty()) }
    var totpSecret by remember(key) { mutableStateOf(existing?.totpSecret.orEmpty()) }
    var notes by remember(key) { mutableStateOf(existing?.notes.orEmpty()) }
    var showPassword by remember { mutableStateOf(false) }
    var isSaving by remember { mutableStateOf(false) }
    var totpSnapshot by remember { mutableStateOf<TotpCodeSnapshot?>(null) }

    LaunchedEffect(totpSecret) {
        val secret = totpSecret.trim()
        if (secret.isEmpty()) {
            totpSnapshot = null
            return@LaunchedEffect
        }
        while (true) {
            totpSnapshot = runSuspendCatching { engine.vaultTotpNow(secret) }.getOrNull()
            delay(1000L)
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

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        properties = ModalBottomSheetProperties(securePolicy = SecureFlagPolicy.SecureOn),
        containerColor = palette.card,
        contentColor = palette.ink,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = if (existing == null) "New login" else "Edit login",
                    style = MaterialTheme.typography.titleMedium,
                    color = palette.ink,
                )
                if (existing != null) {
                    IconButton(
                        onClick = {
                            scope.launch {
                                runSuspendCatching { engine.vaultTrashItem(existing.id) }
                                    .onSuccess {
                                        onDismiss()
                                        feedback.undo("Login moved to trash") {
                                            engine.vaultRestoreItem(existing.id)
                                        }
                                    }
                                    .onFailure { feedback.error(it.message ?: "Could not delete login") }
                            }
                        },
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.DeleteOutline,
                            contentDescription = "Delete login",
                            tint = palette.danger,
                        )
                    }
                }
            }

            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Title", color = palette.muted) },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            OutlinedTextField(
                value = username,
                onValueChange = { username = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Username or email", color = palette.muted) },
                trailingIcon = if (username.isNotBlank()) {
                    {
                        IconButton(onClick = {
                            SensitiveClipboard.copySecret(context, "Username", username)
                            feedback.info("Username copied")
                        }) {
                            Icon(
                                imageVector = Icons.Outlined.ContentCopy,
                                contentDescription = "Copy username",
                                tint = palette.muted,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                    }
                } else {
                    null
                },
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
                ),
                visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                trailingIcon = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        IconButton(onClick = {
                            scope.launch {
                                runSuspendCatching { engine.vaultGeneratePassword(20) }
                                    .onSuccess {
                                        password = it
                                        showPassword = true
                                    }
                                    .onFailure { feedback.error("Could not generate password") }
                            }
                        }) {
                            Icon(
                                imageVector = Icons.Outlined.AutoAwesome,
                                contentDescription = "Generate password",
                                tint = palette.muted,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                        IconButton(onClick = { showPassword = !showPassword }) {
                            Icon(
                                imageVector = if (showPassword) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                contentDescription = if (showPassword) "Hide password" else "Show password",
                                tint = palette.muted,
                                modifier = Modifier.size(18.dp),
                            )
                        }
                        if (password.isNotEmpty()) {
                            IconButton(onClick = {
                                SensitiveClipboard.copySecret(context, "Password", password)
                                feedback.info("Password copied. Clears in 30s.")
                            }) {
                                Icon(
                                    imageVector = Icons.Outlined.ContentCopy,
                                    contentDescription = "Copy password",
                                    tint = palette.ink,
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                        }
                    }
                },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            val liveTotp = totpSnapshot
            if (liveTotp != null) {
                Surface(
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(12.dp),
                    color = palette.recessed,
                    border = BorderStroke(1.dp, palette.border),
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 14.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Column {
                            Text(
                                text = liveTotp.grouped,
                                style = MaterialTheme.typography.titleLarge.copy(
                                    fontWeight = FontWeight.SemiBold,
                                    letterSpacing = 1.sp,
                                    fontFeatureSettings = "tnum",
                                ),
                                color = palette.ink,
                            )
                            Text(
                                text = "${liveTotp.secondsLeft}s remaining",
                                style = MaterialTheme.typography.bodySmall,
                                color = palette.muted,
                            )
                        }
                        IconButton(onClick = {
                            SensitiveClipboard.copySecret(context, "One-time code", liveTotp.code)
                            feedback.info("Code copied")
                        }) {
                            Icon(
                                imageVector = Icons.Outlined.ContentCopy,
                                contentDescription = "Copy code",
                                tint = palette.ink,
                            )
                        }
                    }
                }
            }

            OutlinedTextField(
                value = url,
                onValueChange = { url = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Website", color = palette.muted) },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            OutlinedTextField(
                value = totpSecret,
                onValueChange = { totpSecret = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("Authenticator setup key", color = palette.muted) },
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Password,
                    autoCorrectEnabled = false,
                ),
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                modifier = Modifier.fillMaxWidth(),
                minLines = 2,
                maxLines = 4,
                placeholder = { Text("Notes", color = palette.muted) },
                shape = RoundedCornerShape(12.dp),
                colors = fieldColors,
            )

            Spacer(Modifier.height(4.dp))

            Button(
                enabled = !isSaving && title.isNotBlank(),
                onClick = {
                    isSaving = true
                    scope.launch {
                        runSuspendCatching {
                            engine.vaultSaveLogin(
                                id = existing?.id,
                                title = title.trim(),
                                username = username.trim(),
                                password = password,
                                url = url.trim(),
                                totpSecret = totpSecret.trim(),
                                notes = notes,
                            )
                        }.onSuccess {
                            onDismiss()
                        }.onFailure {
                            isSaving = false
                            feedback.error(it.message ?: "Could not save login")
                        }
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
                Text("Save", style = MaterialTheme.typography.labelLarge)
            }
        }
    }
}

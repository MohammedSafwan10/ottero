package com.nexdark.ottero.core.autofill

import android.app.assist.AssistStructure
import android.os.Build
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.Dataset
import android.service.autofill.Field
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.FillResponse
import android.service.autofill.Presentations
import android.service.autofill.SaveCallback
import android.service.autofill.SaveRequest
import android.view.View
import android.view.autofill.AutofillId
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import com.nexdark.ottero.core.bridge.OtteroEngine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Android AutofillService backed by Ottero's unlocked in-memory Passwords Vault.
 *
 * Never exposes secrets when the vault is locked; only fills when the user
 * has explicitly unlocked Ottero in the current process.
 */
class OtteroAutofillService : AutofillService() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onFillRequest(
        request: FillRequest,
        cancellationSignal: CancellationSignal,
        callback: FillCallback,
    ) {
        val engine = OtteroEngine.currentOrNull()
        if (engine == null) {
            callback.onSuccess(null)
            return
        }
        val structure = request.fillContexts.lastOrNull()?.structure
        if (structure == null) {
            callback.onSuccess(null)
            return
        }

        val parsed = parseStructure(structure)
        if (parsed.usernameId == null && parsed.passwordId == null) {
            callback.onSuccess(null)
            return
        }

        val hint = parsed.webDomain ?: parsed.packageHint.orEmpty()
        val job = scope.launch {
            val matches = runCatching { engine.vaultAutofillMatches(hint) }.getOrDefault(emptyList())
            if (matches.isEmpty()) {
                callback.onSuccess(null)
                return@launch
            }
            val responseBuilder = FillResponse.Builder()
            matches.forEach { entry ->
                val presentation = RemoteViews(packageName, android.R.layout.simple_list_item_1).apply {
                    val label = if (entry.username.isNotBlank()) {
                        "${entry.title} (${entry.username})"
                    } else {
                        entry.title
                    }
                    setTextViewText(android.R.id.text1, label)
                }
                val datasetBuilder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    val presentations = Presentations.Builder()
                        .setMenuPresentation(presentation)
                        .build()
                    Dataset.Builder(presentations).apply {
                        parsed.usernameId?.let { id ->
                            if (entry.username.isNotBlank()) {
                                setField(
                                    id,
                                    Field.Builder().setValue(AutofillValue.forText(entry.username)).build(),
                                )
                            }
                        }
                        parsed.passwordId?.let { id ->
                            if (entry.password.isNotBlank()) {
                                setField(
                                    id,
                                    Field.Builder().setValue(AutofillValue.forText(entry.password)).build(),
                                )
                            }
                        }
                    }
                } else {
                    @Suppress("DEPRECATION")
                    Dataset.Builder().apply {
                        parsed.usernameId?.let { id ->
                            if (entry.username.isNotBlank()) {
                                setValue(id, AutofillValue.forText(entry.username), presentation)
                            }
                        }
                        parsed.passwordId?.let { id ->
                            if (entry.password.isNotBlank()) {
                                setValue(id, AutofillValue.forText(entry.password), presentation)
                            }
                        }
                    }
                }
                runCatching { responseBuilder.addDataset(datasetBuilder.build()) }
            }
            callback.onSuccess(responseBuilder.build())
        }
        cancellationSignal.setOnCancelListener { job.cancel() }
    }

    override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
        callback.onSuccess()
    }

    private data class ParsedForm(
        val usernameId: AutofillId? = null,
        val passwordId: AutofillId? = null,
        val webDomain: String? = null,
        val packageHint: String? = null,
    )

    private fun parseStructure(structure: AssistStructure): ParsedForm {
        var usernameId: AutofillId? = null
        var passwordId: AutofillId? = null
        var webDomain: String? = null
        val pkgHint = structure.activityComponent?.packageName
            ?.split('.')
            ?.lastOrNull()
            ?.takeIf { it.length >= 3 }

        fun visit(node: AssistStructure.ViewNode) {
            if (webDomain == null && !node.webDomain.isNullOrBlank()) {
                webDomain = node.webDomain
            }
            val id = node.autofillId
            val hints = node.autofillHints.orEmpty()
            val idEntry = node.idEntry.orEmpty().lowercase()
            if (id != null) {
                val isPassword = hints.any { it == View.AUTOFILL_HINT_PASSWORD } ||
                    idEntry.contains("password") ||
                    idEntry.contains("passwd")
                val isUsername = hints.any {
                    it == View.AUTOFILL_HINT_USERNAME || it == View.AUTOFILL_HINT_EMAIL_ADDRESS
                } || idEntry.contains("email") || idEntry.contains("username") || idEntry.contains("login")

                if (isPassword && passwordId == null) {
                    passwordId = id
                } else if (isUsername && usernameId == null) {
                    usernameId = id
                }
            }
            for (i in 0 until node.childCount) {
                visit(node.getChildAt(i))
            }
        }

        for (i in 0 until structure.windowNodeCount) {
            structure.getWindowNodeAt(i).rootViewNode?.let(::visit)
        }
        return ParsedForm(usernameId, passwordId, webDomain, pkgHint)
    }
}

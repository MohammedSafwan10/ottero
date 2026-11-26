package com.nexdark.ottero.core.security

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Copies sensitive secrets (passwords, TOTP codes) to the Android clipboard
 * with [ClipDescription.EXTRA_IS_SENSITIVE] on API 33+ and clears the
 * clipboard after 30 seconds ONLY if the user has not replaced it with
 * another clip. Uses [Context.getApplicationContext] to avoid leaking Activities.
 */
object SensitiveClipboard {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var clearJob: Job? = null

    fun copySecret(context: Context, label: String, secret: String) {
        val appContext = context.applicationContext
        val cm = appContext.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val clip = ClipData.newPlainText(label, secret).apply {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                description.extras = PersistableBundle().apply {
                    putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
                }
            }
        }
        cm.setPrimaryClip(clip)
        val copiedTimestamp = cm.primaryClipDescription?.timestamp

        clearJob?.cancel()
        clearJob = scope.launch {
            delay(30_000L)
            val currentDesc = cm.primaryClipDescription
            val stillOurs = currentDesc != null &&
                currentDesc.label?.toString() == label &&
                (copiedTimestamp == null || currentDesc.timestamp == copiedTimestamp)
            if (stillOurs && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                runCatching { cm.clearPrimaryClip() }
            }
        }
    }
}

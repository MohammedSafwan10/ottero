package com.nexdark.ottero.core.update

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import uniffi.ottero_mobile_ffi.MobileUpdateInfo
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * Verified in-app APK downloader & installer for `direct` distribution builds.
 *
 * Security invariants:
 * - Feed metadata (`downloadUrl`, `sha256`, `sizeBytes`) was already verified
 *   against Ottero's pinned `minisign` public key in Rust (`ottero_core::update`).
 * - Downloads strictly over HTTPS into app-private `cacheDir/updates/`.
 * - Streams bytes through SHA-256 and verifies exact byte length + digest
 *   before ever handing the APK to Android's Package Installer.
 */
object AppUpdater {
    private val httpClient: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .followRedirects(true)
        .followSslRedirects(true)
        .build()

    sealed interface InstallResult {
        data object Launched : InstallResult
        data object NeedsPermission : InstallResult
    }

    suspend fun downloadVerified(
        context: Context,
        info: MobileUpdateInfo,
        onProgress: (Float) -> Unit,
    ): File = withContext(Dispatchers.IO) {
        require(info.downloadUrl.startsWith("https://")) {
            "Update URL must use HTTPS"
        }
        val updatesDir = File(context.cacheDir, "updates").apply { mkdirs() }
        // Clean stale APKs before downloading the new one.
        updatesDir.listFiles()?.forEach { it.delete() }

        val target = File(updatesDir, "ottero-${info.version}.apk")
        val request = Request.Builder()
            .url(info.downloadUrl)
            .header("User-Agent", "Ottero/${info.version} (Android)")
            .build()

        val expectedSize = info.sizeBytes.toLong()
        val digest = MessageDigest.getInstance("SHA-256")
        var downloaded = 0L

        try {
            httpClient.newCall(request).execute().use { response ->
                if (!response.isSuccessful) {
                    error("Download failed (HTTP ${response.code})")
                }
                val body = response.body ?: error("Empty update response")
                body.byteStream().use { input ->
                    FileOutputStream(target).use { output ->
                        val buffer = ByteArray(32 * 1024)
                        while (true) {
                            val read = input.read(buffer)
                            if (read <= 0) break
                            downloaded += read
                            if (downloaded > expectedSize) {
                                error("Downloaded file exceeded expected size")
                            }
                            digest.update(buffer, 0, read)
                            output.write(buffer, 0, read)
                            if (expectedSize > 0L) {
                                onProgress((downloaded.toFloat() / expectedSize.toFloat()).coerceIn(0f, 1f))
                            }
                        }
                        output.flush()
                    }
                }
            }
            if (downloaded != expectedSize) {
                error("Download incomplete ($downloaded of $expectedSize bytes)")
            }
            val actualHex = digest.digest().joinToString("") { "%02x".format(it) }
            if (!actualHex.equals(info.sha256, ignoreCase = true)) {
                error("Update SHA-256 verification failed")
            }
            target
        } catch (t: Throwable) {
            target.delete()
            throw t
        }
    }

    fun installApk(context: Context, apkFile: File): InstallResult {
        if (!context.packageManager.canRequestPackageInstalls()) {
            val intent = Intent(
                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:${context.packageName}"),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
            return InstallResult.NeedsPermission
        }
        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            apkFile,
        )
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
        return InstallResult.Launched
    }
}

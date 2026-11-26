package com.nexdark.ottero.core.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONObject
import uniffi.ottero_mobile_ffi.AccountSessionBundle
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Hardware-backed Android Keystore storage for the Ottero Cloud session
 * (`bearer_token` + `account_key`). Never writes plaintext credentials to disk.
 */
object AccountKeystore {

    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val KEY_ALIAS = "ottero_account_session_v1"
    private const val PREFS = "ottero_secure_session"
    private const val KEY_BLOB = "blob"
    private const val KEY_IV = "iv"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"

    private fun getOrCreateKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (ks.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }

        val spec = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()

        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }

    fun save(context: Context, session: AccountSessionBundle) {
        val appContext = context.applicationContext
        val payload = JSONObject().apply {
            put("email", session.email)
            put("account_id", session.accountId)
            put("bearer_token", session.bearerToken)
            put("account_key_b64", session.accountKeyB64)
        }.toString().toByteArray(Charsets.UTF_8)

        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
        val ciphertext = cipher.doFinal(payload)
        payload.fill(0)

        appContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putString(KEY_BLOB, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
            .putString(KEY_IV, Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .apply()
    }

    fun load(context: Context): AccountSessionBundle? {
        val appContext = context.applicationContext
        val prefs = appContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val blobB64 = prefs.getString(KEY_BLOB, null) ?: return null
        val ivB64 = prefs.getString(KEY_IV, null) ?: return null

        return runCatching {
            val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
            val key = ks.getKey(KEY_ALIAS, null) as? SecretKey ?: return null
            val cipher = Cipher.getInstance(TRANSFORMATION)
            val iv = Base64.decode(ivB64, Base64.NO_WRAP)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, iv))
            val plaintext = cipher.doFinal(Base64.decode(blobB64, Base64.NO_WRAP))
            val json = JSONObject(String(plaintext, Charsets.UTF_8))
            plaintext.fill(0)
            AccountSessionBundle(
                email = json.getString("email"),
                accountId = json.getString("account_id"),
                bearerToken = json.getString("bearer_token"),
                accountKeyB64 = json.getString("account_key_b64"),
            )
        }.getOrNull()
    }

    fun clear(context: Context) {
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .clear()
            .apply()
    }
}

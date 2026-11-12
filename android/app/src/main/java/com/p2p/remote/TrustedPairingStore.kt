package com.p2p.remote

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONObject
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Single-device opt-in pairing profile. AndroidKeyStore holds the non-exportable
 * AES key; SharedPreferences contains only AES/GCM authenticated ciphertext.
 * Nothing is saved unless user taps the explicit Save button.
 */
internal class TrustedPairingStore(private val context: Context) {
    data class Profile(
        val relayUrl: String,
        val relayToken: String,
        val sessionKey: String,
        val displayName: String = "家庭电脑",
    )

    private val alias = "P2PRemoteV05UserPairing"
    private val preferences = context.getSharedPreferences("trusted_pairing_v05", Context.MODE_PRIVATE)

    fun hasSaved(): Boolean = preferences.contains("encrypted_profile")

    private fun key(create: Boolean): SecretKey? {
        val store = KeyStore.getInstance("AndroidKeyStore")
        store.load(null)
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        if (!create) return null
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(
                alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            ).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return generator.generateKey()
    }

    fun save(profile: Profile) {
        require(valid(profile)) { "Invalid pairing fields" }
        val plain = JSONObject()
            .put("v", 1)
            .put("name", profile.displayName)
            .put("relay", profile.relayUrl)
            .put("token", profile.relayToken)
            .put("key", profile.sessionKey)
            .toString().toByteArray(Charsets.UTF_8)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, checkNotNull(key(true)))
        cipher.updateAAD("P2PRemoteTrustedV05".toByteArray(Charsets.UTF_8))
        val packet = cipher.iv + cipher.doFinal(plain)
        preferences.edit()
            .putString("encrypted_profile", Base64.encodeToString(packet, Base64.NO_WRAP))
            .apply()
    }

    fun load(): Profile? {
        return try {
        val base64 = preferences.getString("encrypted_profile", null) ?: return null
        val packet = Base64.decode(base64, Base64.DEFAULT)
        if (packet.size !in 29..8192) return null
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE, key(false) ?: return null,
            GCMParameterSpec(128, packet.copyOfRange(0, 12))
        )
        cipher.updateAAD("P2PRemoteTrustedV05".toByteArray(Charsets.UTF_8))
        val json = JSONObject(cipher.doFinal(packet.copyOfRange(12, packet.size)).toString(Charsets.UTF_8))
        if (json.optInt("v") != 1) return null
        val profile = Profile(
            json.getString("relay"), json.getString("token"),
            json.getString("key"), json.optString("name", "家庭电脑")
        )
        profile.takeIf(::valid)
        } catch (_: Exception) {
            null // Fail closed if Keystore key disappeared, data was tampered with or device was migrated.
        }
    }

    fun clear() {
        preferences.edit().remove("encrypted_profile").commit()
        // Keep Keystore key; no plaintext remains, and a fresh save overwrites the blob.
        // For a hard reset, revoke the session key on Windows too.
    }

    private fun valid(profile: Profile): Boolean {
        val relay = profile.relayUrl
        return (relay.startsWith("wss://") || relay.startsWith("https://")) &&
            relay.length <= 512 && !relay.contains('@') && !relay.contains('?') &&
            !relay.contains('#') &&
            profile.relayToken.length in 32..512 &&
            profile.sessionKey.length == 64 &&
            profile.sessionKey.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' } &&
            profile.displayName.length in 1..48
    }
}

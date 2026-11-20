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
 * Android Keystore-backed Relay credentials for the Flutter Android client.
 * Only authenticated ciphertext is written to SharedPreferences.
 */
internal class RelayCredentialStore(context: Context) {
    data class Credentials(val relayUrl: String, val relayToken: String)

    private val preferences = context.getSharedPreferences("p2p_relay_credentials_v1", Context.MODE_PRIVATE)
    private val alias = "P2PRemoteRelayCredentialsV1"

    fun save(relayUrl: String, relayToken: String) {
        val url = normalize(relayUrl)
        require(relayToken.length in 32..512) { "Relay 访问口令长度无效" }
        val plain = JSONObject()
            .put("v", 1)
            .put("relay", url)
            .put("token", relayToken)
            .toString().toByteArray(Charsets.UTF_8)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key(create = true))
        cipher.updateAAD("P2PRemoteRelayCredentialsV1".toByteArray(Charsets.UTF_8))
        val packet = cipher.iv + cipher.doFinal(plain)
        check(preferences.edit()
            .putString("encrypted_credentials", Base64.encodeToString(packet, Base64.NO_WRAP))
            .commit()) { "无法保存 Android Keystore 加密凭据" }
    }

    fun load(): Credentials? = try {
        val encoded = preferences.getString("encrypted_credentials", null) ?: return null
        val packet = Base64.decode(encoded, Base64.DEFAULT)
        if (packet.size !in 29..8192) return null
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(create = false) ?: return null,
            GCMParameterSpec(128, packet.copyOfRange(0, 12))
        )
        cipher.updateAAD("P2PRemoteRelayCredentialsV1".toByteArray(Charsets.UTF_8))
        val json = JSONObject(cipher.doFinal(packet.copyOfRange(12, packet.size)).toString(Charsets.UTF_8))
        if (json.optInt("v") != 1) return null
        val url = normalize(json.getString("relay"))
        val token = json.getString("token")
        if (token.length !in 32..512) return null
        Credentials(url, token)
    } catch (_: Exception) {
        null
    }

    fun clear() {
        preferences.edit().remove("encrypted_credentials").commit()
    }

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

    private fun normalize(value: String): String {
        val input = value.trim().trimEnd('/')
        val result = when {
            input.startsWith("wss://") -> input
            input.startsWith("https://") -> "wss://" + input.removePrefix("https://")
            else -> throw IllegalArgumentException("中转服务器必须使用 HTTPS/WSS")
        }
        require(!result.contains('@') && !result.contains('?') && !result.contains('#')) {
            "中转地址不能包含账号或查询参数"
        }
        return result
    }
}

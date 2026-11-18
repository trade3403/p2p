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

/** Persistent Ed25519 identity seed encrypted by a non-exportable Android Keystore AES key. */
internal class DeviceIdentityStore(context: Context) {
    private val alias = "P2PRemoteDeviceIdentityV014"
    private val preferences = context.getSharedPreferences("device_identity_v014", Context.MODE_PRIVATE)

    fun loadOrCreate(): PairingCrypto.Identity {
        val encoded = preferences.getString("identity_blob", null)
        if (encoded != null) {
            val packet = Base64.decode(encoded, Base64.DEFAULT)
            require(packet.size in 29..4096) { "Stored device identity is invalid" }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.DECRYPT_MODE,
                key(create = false) ?: error("Device identity key is missing; re-pair this device"),
                GCMParameterSpec(128, packet.copyOfRange(0, 12))
            )
            cipher.updateAAD("P2PRemoteDeviceIdentityV014".toByteArray(Charsets.UTF_8))
            val json = JSONObject(cipher.doFinal(packet.copyOfRange(12, packet.size)).toString(Charsets.UTF_8))
            require(json.optInt("version") == 1) { "Unsupported device identity version" }
            val deviceId = json.getString("device_id")
            val seed = Base64.decode(json.getString("signing_seed"), Base64.NO_WRAP)
            require(seed.size == 32 && deviceId.matches(Regex("^[a-f0-9]{32}$"))) {
                "Stored device identity fields are invalid"
            }
            return PairingCrypto.restoreIdentity(deviceId, seed)
        }

        val identity = PairingCrypto.generateIdentity()
        val json = JSONObject()
            .put("version", 1)
            .put("device_id", identity.deviceId)
            .put("signing_seed", Base64.encodeToString(identity.privateSeed, Base64.NO_WRAP))
            .toString().toByteArray(Charsets.UTF_8)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, checkNotNull(key(create = true)))
        cipher.updateAAD("P2PRemoteDeviceIdentityV014".toByteArray(Charsets.UTF_8))
        val packet = cipher.iv + cipher.doFinal(json)
        check(preferences.edit().putString(
            "identity_blob", Base64.encodeToString(packet, Base64.NO_WRAP)
        ).commit()) { "Could not persist device identity" }
        return identity
    }

    fun clear() {
        preferences.edit().remove("identity_blob").commit()
        KeyStore.getInstance("AndroidKeyStore").apply {
            load(null)
            deleteEntry(alias)
        }
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
}

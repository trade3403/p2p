package com.p2p.remote

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.IOException
import java.security.SecureRandom
import java.util.concurrent.TimeUnit

/** HTTPS client for the Relay numeric-pairing coordinator. Never logs OTP or proofs. */
internal class PairingApiClient(server: String, private val relayToken: String) {
    private val baseUrl = normalize(server)
    private val http = OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(8, TimeUnit.SECONDS)
        .writeTimeout(8, TimeUnit.SECONDS)
        .callTimeout(10, TimeUnit.SECONDS)
        .build()
    private val jsonType = "application/json; charset=utf-8".toMediaType()

    fun pairAndDeriveSession(
        deviceNumber: String,
        sixDigitCode: String,
        cancelled: () -> Boolean = { false },
        onProgress: (String) -> Unit = {},
    ): ByteArray {
        require(deviceNumber.matches(Regex("^[0-9]{9}$"))) { "设备号必须是9位数字" }
        require(sixDigitCode.matches(Regex("^[0-9]{6}$"))) { "动态验证码必须是6位数字" }
        require(relayToken.length >= 32) { "请先配置中转服务器访问口令" }

        onProgress("正在查询设备…")
        val device = call("/api/v1/devices/$deviceNumber")
        if (!device.optBoolean("online", false)) throw IOException("目标电脑当前不在线")
        val expiresAt = device.optLong("pairing_expires_at", 0L)
        if (expiresAt <= System.currentTimeMillis() / 1000L) {
            throw IOException("目标电脑的动态验证码已过期，请让对方刷新验证码")
        }
        val identityPublic = PairingCrypto.decodeBase64Url(device.getString("identity_public_key"))
        val pairingPublicEncoded = device.getString("pairing_public_key")
        val pairingPublic = PairingCrypto.decodeBase64Url(pairingPublicEncoded)
        val signature = device.getString("pairing_signature")
        if (!PairingCrypto.verifyPairingWindow(
                identityPublic, deviceNumber, expiresAt, pairingPublicEncoded, signature
            )
        ) {
            throw IOException("设备身份签名验证失败，已拒绝连接")
        }

        val viewer = PairingCrypto.generateViewerEphemeral()
        val nonce = ByteArray(16).also(SecureRandom()::nextBytes)
        val proof = PairingCrypto.proofForViewer(
            viewer, deviceNumber, pairingPublic, nonce, sixDigitCode
        )
        val viewerPublicEncoded = PairingCrypto.publicKeyBase64(viewer.publicKey)
        val nonceEncoded = PairingCrypto.publicKeyBase64(nonce)
        val createBody = JSONObject()
            .put("device_number", deviceNumber)
            .put("viewer_public_key", viewerPublicEncoded)
            .put("nonce", nonceEncoded)
            .put("proof", PairingCrypto.publicKeyBase64(proof))

        onProgress("验证码已提交，等待电脑端确认…")
        val created = call("/api/v1/pairing/requests", createBody)
        val requestId = created.getString("request_id")
        val requestExpiry = created.optLong("expires_at", expiresAt)
        while (!cancelled() && System.currentTimeMillis() / 1000L < requestExpiry) {
            val result = call("/api/v1/pairing/requests/$requestId")
            when (result.optString("status")) {
                "approved" -> {
                    onProgress("电脑端已批准，正在建立加密会话…")
                    return PairingCrypto.sessionSecretForViewer(
                        viewer, deviceNumber, pairingPublic, nonce
                    )
                }
                "denied" -> throw IOException("验证码错误、已过期，或电脑端拒绝了本次连接")
                "expired" -> throw IOException("配对请求已过期，请重新输入验证码")
                "pending" -> onProgress("等待电脑端用户确认…")
                else -> throw IOException("中转服务器返回未知配对状态")
            }
            Thread.sleep(900L)
        }
        if (cancelled()) throw IOException("已取消配对")
        throw IOException("等待电脑端确认超时，请重新发起配对")
    }

    private fun call(path: String, body: JSONObject? = null): JSONObject {
        val builder = Request.Builder()
            .url(baseUrl + path)
            .header("Authorization", "Bearer $relayToken")
            .header("Accept", "application/json")
        if (body != null) {
            builder.post(body.toString().toRequestBody(jsonType))
        } else {
            builder.get()
        }
        http.newCall(builder.build()).execute().use { response ->
            val text = response.body?.string().orEmpty()
            if (!response.isSuccessful) {
                val message = runCatching { JSONObject(text).optString("error") }.getOrNull()
                throw IOException(message?.takeIf { it.isNotBlank() } ?: ("配对服务请求失败：HTTP " + response.code))
            }
            return JSONObject(text.ifBlank { "{}" })
        }
    }

    private fun normalize(value: String): String {
        val input = value.trim().trimEnd('/')
        val normalized = when {
            input.startsWith("wss://") -> "https://" + input.removePrefix("wss://")
            input.startsWith("https://") -> input
            else -> throw IllegalArgumentException("中转服务器必须使用 HTTPS/WSS")
        }
        require(!normalized.contains('@') && !normalized.contains('?') && !normalized.contains('#')) {
            "中转地址不能包含账号或查询参数"
        }
        return normalized
    }
}

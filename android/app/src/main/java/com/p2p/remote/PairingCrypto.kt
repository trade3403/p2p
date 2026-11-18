package com.p2p.remote

import org.bouncycastle.crypto.digests.SHA256Digest
import org.bouncycastle.crypto.generators.HKDFBytesGenerator
import org.bouncycastle.crypto.macs.HMac
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.params.HKDFParameters
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.crypto.params.X25519PrivateKeyParameters
import org.bouncycastle.crypto.params.X25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import java.security.SecureRandom
import java.util.Arrays
import java.util.Base64

/** Must remain byte-for-byte compatible with desktop/src/pairing_crypto.rs. */
internal object PairingCrypto {
    private const val WINDOW_DOMAIN = "p2p-pairing-window-v1|"
    private const val PROOF_DOMAIN = "p2p-pairing-proof-v1|"
    private const val REQUEST_DOMAIN = "p2p-pairing-request-v1|"
    private const val DECISION_DOMAIN = "p2p-pairing-decision-v1|"
    private const val SESSION_DOMAIN = "p2p-remote-session-v1|"
    private val random = SecureRandom()

    data class Identity(val deviceId: String, val privateSeed: ByteArray, val publicKey: ByteArray)
    data class PairingWindow(
        internal val privateKey: X25519PrivateKeyParameters,
        val publicKey: ByteArray,
        val code: String,
        val expiresAt: Long,
    )
    data class ViewerEphemeral(
        internal val privateKey: X25519PrivateKeyParameters,
        val publicKey: ByteArray,
    )

    fun generateIdentity(): Identity {
        val deviceId = ByteArray(16).also(random::nextBytes)
        val seed = ByteArray(32).also(random::nextBytes)
        val privateKey = Ed25519PrivateKeyParameters(seed, 0)
        return Identity(deviceId.toHex(), seed, privateKey.generatePublicKey().encoded)
    }

    fun generatePairingWindow(nowUnix: Long): PairingWindow {
        val privateKey = X25519PrivateKeyParameters(random)
        val code = random.nextInt(1_000_000).toString().padStart(6, '0')
        return PairingWindow(privateKey, privateKey.generatePublicKey().encoded, code, nowUnix + 120)
    }

    fun generateViewerEphemeral(): ViewerEphemeral {
        val privateKey = X25519PrivateKeyParameters(random)
        return ViewerEphemeral(privateKey, privateKey.generatePublicKey().encoded)
    }

    fun publicKeyBase64(bytes: ByteArray): String = b64(bytes)

    fun signPairingWindow(
        signingSeed: ByteArray, deviceNumber: String, expiresAt: Long, pairingPublicBase64: String
    ): String {
        require(signingSeed.size == 32)
        val message = "$WINDOW_DOMAIN$deviceNumber|$expiresAt|$pairingPublicBase64".toByteArray(Charsets.UTF_8)
        val signer = Ed25519Signer()
        signer.init(true, Ed25519PrivateKeyParameters(signingSeed, 0))
        signer.update(message, 0, message.size)
        return b64(signer.generateSignature())
    }

    fun verifyPairingWindow(
        identityPublicKey: ByteArray,
        deviceNumber: String,
        expiresAt: Long,
        pairingPublicBase64: String,
        signatureBase64: String,
    ): Boolean {
        if (identityPublicKey.size != 32) return false
        return try {
            val signature = decodeB64(signatureBase64)
            if (signature.size != 64) return false
            val message = "$WINDOW_DOMAIN$deviceNumber|$expiresAt|$pairingPublicBase64".toByteArray(Charsets.UTF_8)
            val verifier = Ed25519Signer()
            verifier.init(false, Ed25519PublicKeyParameters(identityPublicKey, 0))
            verifier.update(message, 0, message.size)
            verifier.verifySignature(signature)
        } catch (_: Exception) { false }
    }

    fun proofForViewer(
        viewer: ViewerEphemeral,
        deviceNumber: String,
        hostPairingPublicKey: ByteArray,
        nonce: ByteArray,
        sixDigitCode: String,
    ): ByteArray {
        require(nonce.size == 16 && hostPairingPublicKey.size == 32)
        val shared = ByteArray(32)
        viewer.privateKey.generateSecret(X25519PublicKeyParameters(hostPairingPublicKey, 0), shared, 0)
        return try {
            makeProof(shared, deviceNumber, sixDigitCode, viewer.publicKey, nonce)
        } finally { Arrays.fill(shared, 0) }
    }

    fun proofForHost(
        host: PairingWindow,
        deviceNumber: String,
        viewerPublicKey: ByteArray,
        nonce: ByteArray,
    ): ByteArray {
        require(nonce.size == 16 && viewerPublicKey.size == 32)
        val shared = ByteArray(32)
        host.privateKey.generateSecret(X25519PublicKeyParameters(viewerPublicKey, 0), shared, 0)
        return try {
            makeProof(shared, deviceNumber, host.code, viewerPublicKey, nonce)
        } finally { Arrays.fill(shared, 0) }
    }

    fun sessionSecretForViewer(
        viewer: ViewerEphemeral, deviceNumber: String, hostPairingPublicKey: ByteArray, nonce: ByteArray
    ): ByteArray {
        require(nonce.size == 16 && hostPairingPublicKey.size == 32)
        val shared = ByteArray(32)
        viewer.privateKey.generateSecret(X25519PublicKeyParameters(hostPairingPublicKey, 0), shared, 0)
        return try { deriveSession(shared, deviceNumber, nonce) } finally { Arrays.fill(shared, 0) }
    }

    fun sessionSecretForHost(
        host: PairingWindow, deviceNumber: String, viewerPublicKey: ByteArray, nonce: ByteArray
    ): ByteArray {
        require(nonce.size == 16 && viewerPublicKey.size == 32)
        val shared = ByteArray(32)
        host.privateKey.generateSecret(X25519PublicKeyParameters(viewerPublicKey, 0), shared, 0)
        return try { deriveSession(shared, deviceNumber, nonce) } finally { Arrays.fill(shared, 0) }
    }

    fun signDecision(
        signingSeed: ByteArray, requestId: String, deviceNumber: String,
        viewerPublicBase64: String, nonceBase64: String, approved: Boolean
    ): String {
        require(signingSeed.size == 32)
        val decision = if (approved) "approved" else "denied"
        val message = "$DECISION_DOMAIN$requestId|$deviceNumber|$viewerPublicBase64|$nonceBase64|$decision"
            .toByteArray(Charsets.UTF_8)
        val signer = Ed25519Signer()
        signer.init(true, Ed25519PrivateKeyParameters(signingSeed, 0))
        signer.update(message, 0, message.size)
        return b64(signer.generateSignature())
    }

    private fun makeProof(
        shared: ByteArray, deviceNumber: String, code: String, viewerPublic: ByteArray, nonce: ByteArray
    ): ByteArray {
        require(code.length == 6 && code.all(Char::isDigit))
        require(shared.any { it.toInt() != 0 }) { "invalid all-zero X25519 shared secret" }
        val info = "$PROOF_DOMAIN$deviceNumber|$code".toByteArray(Charsets.UTF_8)
        val hkdf = HKDFBytesGenerator(SHA256Digest())
        hkdf.init(HKDFParameters(shared, nonce, info))
        val key = ByteArray(32)
        hkdf.generateBytes(key, 0, key.size)
        return try {
            val message = REQUEST_DOMAIN + deviceNumber + "|" + b64(viewerPublic) + "|" + b64(nonce)
            val bytes = message.toByteArray(Charsets.UTF_8)
            val mac = HMac(SHA256Digest())
            mac.init(KeyParameter(key))
            mac.update(bytes, 0, bytes.size)
            ByteArray(mac.macSize).also { mac.doFinal(it, 0) }
        } finally { Arrays.fill(key, 0) }
    }

    private fun deriveSession(shared: ByteArray, deviceNumber: String, nonce: ByteArray): ByteArray {
        require(shared.any { it.toInt() != 0 }) { "invalid all-zero X25519 shared secret" }
        val info = "$SESSION_DOMAIN$deviceNumber".toByteArray(Charsets.UTF_8)
        val hkdf = HKDFBytesGenerator(SHA256Digest())
        hkdf.init(HKDFParameters(shared, nonce, info))
        return ByteArray(32).also { hkdf.generateBytes(it, 0, it.size) }
    }

    private fun b64(value: ByteArray): String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(value)

    private fun decodeB64(value: String): ByteArray = Base64.getUrlDecoder().decode(value)

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
}

package com.p2p.remote

import java.security.SecureRandom

/** One handshake transcript per WebSocket; methods are called on the main thread.
 * All payloads are authenticated by the existing AES-GCM packet codec.
 */
internal class LanHandshake(private val clientNonce: ByteArray = ByteArray(32).also {
    SecureRandom().nextBytes(it)
}) {
    private val client = clientNonce.copyOf().also { require(it.size == 32) }
    private var server: ByteArray? = null
    private var complete = false

    fun hello(): ByteArray = prefix("HELLO") + client

    fun acceptChallenge(payload: ByteArray): ByteArray? {
        if (server != null || complete) return null
        val expected = prefix("CHALLENGE") + client
        if (payload.size != expected.size + 32 ||
            !payload.copyOfRange(0, expected.size).contentEquals(expected)) return null
        val nonce = payload.copyOfRange(expected.size, payload.size)
        server = nonce
        return prefix("PROOF") + client + nonce
    }

    fun acceptReady(payload: ByteArray): Boolean {
        val nonce = server ?: return false
        if (complete || !payload.contentEquals(prefix("READY") + client + nonce)) return false
        complete = true
        return true
    }

    fun businessKey(root: ByteArray): ByteArray {
        check(complete) { "LAN handshake not complete" }
        val mac = javax.crypto.Mac.getInstance("HmacSHA256")
        mac.init(javax.crypto.spec.SecretKeySpec(root, "HmacSHA256"))
        mac.update("P2P-LAN-BUSINESS-v4".toByteArray(Charsets.UTF_8))
        mac.update(client)
        return mac.doFinal(requireNotNull(server))
    }

    private fun prefix(kind: String) = "P2P-LAN-$kind-v4".toByteArray(Charsets.UTF_8)
}

/** LAN v4: independent directions, fixed-size state, no nonce reuse.
 * All outgoing messages are serialized on the UI thread.
 */
internal class LanSequence {
    private var sent = 0L
    private var received = 0L

    @Synchronized fun nextNonce(): ByteArray {
        check(sent < MAX_SEQUENCE) { "LAN sequence exhausted" }
        sent++
        return java.nio.ByteBuffer.allocate(12).putInt(VIEW).putLong(sent).array()
    }

    // Call only after AES-GCM authentication.
    @Synchronized fun accept(packet: ByteArray): Boolean {
        if (packet.size < 13) return false
        val nonce = java.nio.ByteBuffer.wrap(packet, 1, 12)
        if (nonce.int != HOST) return false
        val sequence = nonce.long
        if (sequence <= received || sequence > MAX_SEQUENCE) return false
        received = sequence
        return true
    }

    companion object {
        private const val HOST = 0x484f5354
        private const val VIEW = 0x56494557
        private const val MAX_SEQUENCE = 0x7fff_ffffL
    }
}

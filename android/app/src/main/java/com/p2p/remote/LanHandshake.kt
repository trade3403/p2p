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
        mac.update("P2P-LAN-BUSINESS-v3".toByteArray(Charsets.UTF_8))
        mac.update(client)
        return mac.doFinal(requireNotNull(server))
    }

    private fun prefix(kind: String) = "P2P-LAN-$kind-v3".toByteArray(Charsets.UTF_8)
}

/** Per-socket authenticated business nonce guard. Called only after AES-GCM
 * verification. No eviction: duplicates remain rejected until disconnect.
 * Compact fixed-width representation avoids retaining frame payloads.
 */
internal class LanReplayGuard(private val limit: Int = 1_000_000) {
    private data class Nonce(val first: Long, val last: Int)
    private val seen = HashSet<Nonce>()
    val full: Boolean get() = seen.size >= limit

    fun accept(packet: ByteArray): Boolean {
        if (packet.size < 13 || full) return false
        val nonce = java.nio.ByteBuffer.wrap(packet, 1, 12)
        return seen.add(Nonce(nonce.long, nonce.int))
    }
}

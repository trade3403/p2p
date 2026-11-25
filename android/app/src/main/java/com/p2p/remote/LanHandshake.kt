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

    private fun prefix(kind: String) = "P2P-LAN-$kind-v2".toByteArray(Charsets.UTF_8)
}

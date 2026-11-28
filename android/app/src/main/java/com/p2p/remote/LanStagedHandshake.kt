package com.p2p.remote

import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/** Separate v5 transcript. No business key until explicit commit and READY. */
internal class LanStagedHandshake(clientNonce: ByteArray = ByteArray(32).also {
    SecureRandom().nextBytes(it)
}) {
    private enum class Stage { HELLO, PROOF, PREPARED, COMMIT, READY }
    private var stage = Stage.HELLO
    private val client = clientNonce.copyOf().also { require(it.size == 32) }
    private var server: ByteArray? = null
    private fun prefix(kind: String) = "P2P-LAN-$kind-v5".toByteArray(Charsets.UTF_8)
    private fun transcript(kind: String) = prefix(kind) + client + requireNotNull(server)

    fun hello() = prefix("HELLO") + client

    fun acceptChallenge(payload: ByteArray): ByteArray? {
        if (stage != Stage.HELLO) return null
        val expected = prefix("CHALLENGE") + client
        if (payload.size != expected.size + 32 ||
            !payload.copyOfRange(0, expected.size).contentEquals(expected)) return null
        server = payload.copyOfRange(expected.size, payload.size)
        stage = Stage.PROOF
        return transcript("PROOF")
    }

    fun acceptPrepared(payload: ByteArray): Boolean {
        if (stage != Stage.PROOF || !payload.contentEquals(transcript("PREPARED"))) return false
        stage = Stage.PREPARED
        return true
    }

    fun commit(): ByteArray? {
        if (stage != Stage.PREPARED) return null
        stage = Stage.COMMIT
        return transcript("COMMIT")
    }

    fun acceptReady(payload: ByteArray): Boolean {
        if (stage != Stage.COMMIT || !payload.contentEquals(transcript("READY"))) return false
        stage = Stage.READY
        return true
    }

    fun businessKey(root: ByteArray): ByteArray {
        check(stage == Stage.READY)
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(root, "HmacSHA256"))
        mac.update(prefix("BUSINESS"))
        mac.update(client)
        return mac.doFinal(requireNotNull(server))
    }
}

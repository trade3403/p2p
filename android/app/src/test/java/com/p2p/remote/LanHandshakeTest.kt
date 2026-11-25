package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class LanHandshakeTest {
    private fun prefix(kind: String) = "P2P-LAN-$kind-v4".toByteArray(Charsets.UTF_8)

    @Test fun orderedNoncesRejectReplayAndWrongDirection() {
        val sequence = LanSequence()
        val first = java.nio.ByteBuffer.wrap(sequence.nextNonce())
        assertEquals(0x56494557, first.int)
        assertEquals(1L, first.long)
        val second = java.nio.ByteBuffer.wrap(sequence.nextNonce())
        assertEquals(0x56494557, second.int)
        assertEquals(2L, second.long)
        fun packet(direction: Int, count: Long) =
            java.nio.ByteBuffer.allocate(13).put(1.toByte()).putInt(direction).putLong(count).array()
        assertFalse(sequence.accept(packet(0x56494557, 1)))
        assertTrue(sequence.accept(packet(0x484f5354, 1)))
        assertFalse(sequence.accept(packet(0x484f5354, 1)))
        assertTrue(sequence.accept(packet(0x484f5354, 1_000_001)))
        assertFalse(sequence.accept(packet(0x484f5354, 2)))
        assertFalse(sequence.accept(packet(0x484f5354, 0x8000_0000L)))
        assertTrue(sequence.accept(packet(0x484f5354, 1_000_002)))
    }

    @Test fun businessKeyChangesWithEachHostChallenge() {
        val root = ByteArray(32) { 7 }
        val client = ByteArray(32) { 1 }
        fun key(host: ByteArray): ByteArray {
            val state = LanHandshake(client)
            assertNotNull(state.acceptChallenge(prefix("CHALLENGE") + client + host))
            assertTrue(state.acceptReady(prefix("READY") + client + host))
            return state.businessKey(root)
        }
        val first = key(ByteArray(32) { 2 })
        assertEquals(32, first.size)
        assertArrayEquals(first, key(ByteArray(32) { 2 }))
        assertFalse(first.contentEquals(key(ByteArray(32) { 3 })))
        assertFalse(first.contentEquals(root))
    }

    @Test fun freshTranscriptRejectsReplayAndLegacyReady() {
        val client = ByteArray(32) { 1 }
        val host = ByteArray(32) { 2 }
        val state = LanHandshake(client)
        assertArrayEquals(prefix("HELLO") + client, state.hello())
        assertFalse(state.acceptReady(prefix("READY") + client + host))
        assertNull(state.acceptChallenge(prefix("CHALLENGE") + ByteArray(32) { 3 } + host))
        assertArrayEquals(prefix("PROOF") + client + host,
            state.acceptChallenge(prefix("CHALLENGE") + client + host))
        assertNull(state.acceptChallenge(prefix("CHALLENGE") + client + host))
        assertFalse(state.acceptReady("P2P-LAN-READY-v1".toByteArray()))
        assertFalse(state.acceptReady(prefix("READY") + client + ByteArray(32) { 4 }))
        assertTrue(state.acceptReady(prefix("READY") + client + host))
        assertFalse(state.acceptReady(prefix("READY") + client + host))

        val fresh = LanHandshake(ByteArray(32) { 5 })
        assertNull(fresh.acceptChallenge(prefix("CHALLENGE") + client + host))
        assertFalse(fresh.acceptReady(prefix("READY") + client + host))
    }
}

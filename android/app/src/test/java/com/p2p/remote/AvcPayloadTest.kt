package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class AvcPayloadTest {
    @Test fun acceptsBoundedAnnexBFrame() {
        val header = byteArrayOf(0x05, 0x00, 0x02, 0xD0.toByte(), 0, 0, 0, 1, 0x65)
        val frame = AvcPayload.parse(header)
        assertNotNull(frame)
        assertEquals(1280, frame!!.width)
        assertEquals(720, frame.height)
        assertEquals(header.size, frame.bytes.size)
    }

    @Test fun acceptsFullHdAndRejectsBeyondNegotiationCeiling() {
        fun packet(w:Int,h:Int)=byteArrayOf((w shr 8).toByte(),w.toByte(),
            (h shr 8).toByte(),h.toByte(),0,0,0,1,0x65)
        val fullHd=requireNotNull(AvcPayload.parse(packet(1920,1080)))
        assertEquals(1920,fullHd.width)
        assertEquals(1080,fullHd.height)
        assertNull(AvcPayload.parse(packet(1922,1080)))
        assertNull(AvcPayload.parse(packet(1920,1082)))
        assertNull(AvcPayload.parse(packet(1920,1079)))
    }

    @Test fun refusesOddOrMalformedFrames() {
        assertNull(AvcPayload.parse(byteArrayOf(0, 0, 0)))
        assertNull(AvcPayload.parse(byteArrayOf(0x05, 1, 2, 3, 0, 0, 0, 1, 0x65)))
        assertNull(AvcPayload.parse(byteArrayOf(0x05, 0, 0x02, 0xD0.toByte(), 2, 3, 4, 5, 6)))
    }

    private fun unit(vararg nals: Int): AvcPayload =
        requireNotNull(AvcPayload.parse(byteArrayOf(5,0,2,0xD0.toByte()) +
            nals.map { it.toByte() }.toByteArray()))

    @Test fun recoveryRequiresSpsPpsBeforeIdr() {
        assertTrue(unit(0,0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertTrue(unit(0,0,1,0x09,0x10,0,0,1,0x67,0x80,0,0,0,1,0x68,0x80,
            0,0,1,0x06,0x80,0,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x67,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x68,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x65,0x80,0,0,1,0x67,0x80,0,0,1,0x68,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x61,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
    }

    @Test fun recoveryRejectsTruncatedOrForbiddenNalHeaders() {
        assertFalse(unit(0,0,1,0x67,0,0,0,1,0x68,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x65).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0xE7,0x80,0,0,1,0x68,0x80,0,0,1,0x65,0x80).hasSelfContainedIdr())
        assertFalse(unit(0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1).hasSelfContainedIdr())
    }
}

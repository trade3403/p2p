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

    @Test fun refusesOddOrMalformedFrames() {
        assertNull(AvcPayload.parse(byteArrayOf(0, 0, 0)))
        assertNull(AvcPayload.parse(byteArrayOf(0x05, 1, 2, 3, 0, 0, 0, 1, 0x65)))
        assertNull(AvcPayload.parse(byteArrayOf(0x05, 0, 0x02, 0xD0.toByte(), 2, 3, 4, 5, 6)))
    }
}

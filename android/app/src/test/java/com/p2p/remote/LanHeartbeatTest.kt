package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class LanHeartbeatTest {
    @Test fun matchingPongRearmsWithoutExtendingPendingDeadline() {
        var next = 10L
        val h = LanHeartbeat { next++ }
        h.reset(100)
        assertEquals(LanHeartbeat.Ping(10,100),h.poll(100))
        assertNull(h.pong(99,500))
        assertNull(h.poll(5000))
        assertFalse(h.expired(12099))
        assertTrue(h.expired(12100))
        assertNull(h.pong(10,12100))
        h.reset(20000)
        assertEquals(LanHeartbeat.Ping(11,20000),h.poll(20000))
        assertEquals(30L,h.pong(11,20030))
        assertNull(h.pong(11,21000))
        assertNull(h.poll(24029))
        assertEquals(LanHeartbeat.Ping(12,24030),h.poll(24030))
        assertNull(h.pong(11,24031))
        assertEquals(10L,h.pong(12,24040))
    }
}

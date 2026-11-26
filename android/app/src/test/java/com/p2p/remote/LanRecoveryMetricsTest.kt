package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class LanRecoveryMetricsTest {
    @Test fun verifiedThenFallbackMeasuresOpenAndFirstVideoSeparately() {
        val m = LanRecoveryMetrics()
        m.beginLan(100)
        m.verified(250)
        assertEquals(150L, m.attemptMs)
        m.failed("peer_closed", true, 1000)
        assertNull(m.relayOpenMs)
        assertNull(m.firstVideoMs)
        m.relayVideo(1100)
        assertNull(m.firstVideoMs)
        m.relayOpened(1400)
        assertEquals(400L, m.relayOpenMs)
        assertNull(m.firstVideoMs)
        m.relayVideo(1800)
        assertEquals(800L, m.firstVideoMs)
        m.relayOpened(2500)
        m.relayVideo(2600)
        assertEquals(400L, m.relayOpenMs)
        assertEquals(800L, m.firstVideoMs)
    }

    @Test fun unsuccessfulLanAndMissingFallbackAreNotReportedAsRecovery() {
        val m = LanRecoveryMetrics()
        m.beginLan(100)
        m.failed("connect_or_verify_timeout", false, 4100)
        assertEquals(4000L, m.attemptMs)
        assertEquals("connect_or_verify_timeout", m.reason)
        assertFalse(m.fallbackStarted)
        m.relayOpened(4200)
        m.relayVideo(4300)
        assertNull(m.relayOpenMs)
        assertNull(m.firstVideoMs)
        m.reset()
        assertNull(m.reason)
        assertNull(m.attemptMs)
        assertFalse(m.fallbackStarted)
    }
}

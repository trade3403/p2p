package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class ReconnectPolicyTest {
    @Test fun lanFallbackCannotLoopUntilUserStartsNewSession() {
        val policy = LanAttemptPolicy()
        assertFalse(policy.take(""))
        assertFalse(policy.attempted)
        assertTrue(policy.take("ws://192.168.1.2:43821/direct"))
        assertTrue(policy.attempted)
        assertFalse(policy.canFallback(""))
        assertFalse(policy.canFallback("x".repeat(31)))
        assertTrue(policy.canFallback("x".repeat(32)))
        repeat(20) { assertFalse(policy.take("ws://192.168.1.3:43821/direct")) }
        policy.reset()
        assertTrue(policy.take("ws://192.168.1.2:43821/direct"))
    }

    @Test fun openingRequiresRequestedSessionAndNoActiveSocket() {
        assertFalse(LanAttemptPolicy.canOpen(false, false))
        assertFalse(LanAttemptPolicy.canOpen(false, true))
        assertFalse(LanAttemptPolicy.canOpen(true, true))
        assertTrue(LanAttemptPolicy.canOpen(true, false))
        assertEquals(4000L, LanAttemptPolicy.CONNECT_TIMEOUT_MS)
    }

    @Test fun usesBoundedBackoff() {
        listOf(1000L, 2000L, 4000L, 8000L, 16000L, 30000L, 30000L)
            .forEachIndexed { attempt, expected ->
                assertEquals(expected, ReconnectPolicy.delayMs(attempt))
            }
        assertEquals(1000L, ReconnectPolicy.delayMs(-1))
        assertEquals(30000L, ReconnectPolicy.delayMs(Int.MAX_VALUE))
    }
}

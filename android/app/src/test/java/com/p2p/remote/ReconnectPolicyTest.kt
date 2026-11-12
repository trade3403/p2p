package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Test

class ReconnectPolicyTest {
    @Test fun usesBoundedBackoff() {
        listOf(1000L, 2000L, 4000L, 8000L, 16000L, 30000L, 30000L)
            .forEachIndexed { attempt, expected ->
                assertEquals(expected, ReconnectPolicy.delayMs(attempt))
            }
        assertEquals(1000L, ReconnectPolicy.delayMs(-1))
        assertEquals(30000L, ReconnectPolicy.delayMs(Int.MAX_VALUE))
    }
}

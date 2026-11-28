package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class LanReadmissionPolicyTest {
    @Test fun requiresFailureHealthyRelayStableCandidatesAndCooldown() {
        val p = LanReadmissionPolicy()
        p.observe(0,true,listOf("a"))
        assertEquals("idle",p.state(0))
        p.lanFailed(100)
        p.observe(200,false,listOf("a"))
        assertEquals("waiting_candidates",p.state(200))
        p.observe(300,true,listOf("a"))
        p.observe(5_000,true,listOf("a"))
        assertEquals("cooldown",p.state(10_099))
        assertNull(p.reserve(10_099))
        p.observe(10_100,true,listOf("a"))
        assertEquals("eligible",p.state(10_100))
        val r = requireNotNull(p.reserve(10_100))
        assertEquals(listOf("a"),r.candidates)
        assertNull(p.reserve(10_101))
        assertTrue(p.complete(r,10_200,false))
        assertFalse(p.complete(r,10_300,true))
        assertEquals(1,p.reservations)
        p.observe(10_300,true,listOf("a"))
        p.observe(17_000,true,listOf("a"))
        p.observe(24_000,true,listOf("a"))
        assertNull(p.reserve(30_199))
        assertNotNull(p.reserve(30_200))
    }

    @Test fun networkChangesAndCandidateChangesInvalidateStaleReservations() {
        val p = LanReadmissionPolicy()
        p.lanFailed(0)
        p.observe(1,true,listOf("a","a","b"))
        p.observe(5_000,true,listOf("a","b"))
        val first = requireNotNull(p.reserve(10_000))
        assertEquals(listOf("a","b"),first.candidates)
        p.networkChanged()
        assertFalse(p.isCurrent(first))
        assertFalse(p.complete(first,10_001,true))
        p.observe(10_002,true,listOf("a","b"))
        assertNull(p.reserve(13_001))
        val second = requireNotNull(p.reserve(13_002))
        p.observe(13_003,true,listOf("c"))
        assertFalse(p.isCurrent(second))
        assertFalse(p.complete(second,13_004,true))
        assertEquals(2,p.reservations)
        assertNull(p.reserve(16_002))
        p.reset()
        assertEquals("idle",p.state(90_000))
        assertEquals(0,p.reservations)
    }

    @Test fun expiredAdvertisementCannotReserveWithoutFreshStableObservation() {
        val p = LanReadmissionPolicy()
        p.lanFailed(0)
        p.observe(1,true,listOf("a"))
        assertNull(p.reserve(10_000))
        p.observe(10_001,true,listOf("a"))
        assertNull(p.reserve(13_000))
        assertNotNull(p.reserve(13_001))
    }

    @Test fun repeatedFailuresHaveFiniteSessionBudgetAndSuccessStopsAdmission() {
        val p = LanReadmissionPolicy()
        p.lanFailed(0)
        var now=100_000L
        repeat(LanReadmissionPolicy.MAX_BATCHES) {
            p.observe(now,true,listOf("a"))
            val r=requireNotNull(p.reserve(now+3000))
            assertTrue(p.complete(r,now+4000,false))
            now+=100_000
        }
        p.networkChanged()
        p.observe(now,true,listOf("b"))
        assertEquals("budget_exhausted",p.state(now+100_000))
        assertNull(p.reserve(now+100_000))
        p.reset()
        p.lanFailed(now)
        p.observe(now,true,listOf("a"))
        p.observe(now+5000,true,listOf("a"))
        val r=requireNotNull(p.reserve(now+10_000))
        assertTrue(p.complete(r,now+10_001,true))
        assertEquals("idle",p.state(now+100_000))
    }
}

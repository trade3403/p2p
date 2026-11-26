package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class LanCandidatePlanTest {
    @Test fun triesNextDistinctCandidateWithinOneAbsoluteBudget() {
        val p = LanCandidatePlan()
        p.begin(listOf("a","a","b","c","d"),100L)
        assertEquals(LanCandidatePlan.Attempt("a",4100L),p.take(100L){true})
        assertEquals(LanCandidatePlan.Attempt("b",8100L),p.take(4100L){true})
        assertNull(p.take(8100L){true})
        assertEquals(2,p.attemptCount)
        p.begin(listOf("d"),8200L)
        assertNull(p.take(8200L){true})
    }

    @Test fun rapidFailuresStillCannotExceedThreeAttempts() {
        val p = LanCandidatePlan()
        p.begin(listOf("a","b","c","d","e"),0)
        assertEquals("a",p.take(0){true}?.address)
        assertEquals("b",p.take(100){true}?.address)
        assertEquals("c",p.take(200){true}?.address)
        assertNull(p.take(300){true})
        assertEquals(3,p.attemptCount)
    }

    @Test fun eligibilityIsRecheckedAndVerificationEndsTheBatch() {
        val p = LanCandidatePlan()
        p.begin(listOf("old","good","other"),0)
        assertEquals("good",p.take(1){it!="old"}?.address)
        p.verified()
        assertNull(p.take(2){true})
        assertFalse(p.canFallback(""))
        assertTrue(p.canFallback("x".repeat(32)))
        p.reset()
        assertFalse(p.attempted)
        assertEquals(0,p.attemptCount)
        p.begin(listOf("new"),100)
        assertEquals("new",p.take(100){true}?.address)
    }
}

package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class FullHdPolicyTest {
    @Test fun oneAttemptAndTimeoutUntilNewSocket() {
        val p=FullHdPolicy()
        assertTrue(p.request(1000))
        assertFalse(p.request(2000))
        p.rendered(false)
        assertFalse(p.expired(12999))
        assertTrue(p.expired(13000))
        assertTrue(p.downgrade())
        assertFalse(p.downgrade())
        assertFalse(p.request(14000))
        assertTrue(p.rejectRetiredFormat(1920,1080))
        assertFalse(p.rejectRetiredFormat(1280,720))
        p.rendered(true)
        assertEquals(FullHdPolicy.State.DOWNGRADED,p.state)
        p.reset()
        assertTrue(p.request(15000))
    }

    @Test fun onlyHighResolutionOutputConfirmsAndErrorCanDowngrade() {
        val p=FullHdPolicy()
        p.rendered(true)
        assertEquals(FullHdPolicy.State.LEGACY,p.state)
        assertTrue(p.request(100))
        assertTrue(p.requiresHardware())
        p.rendered(true)
        assertEquals(FullHdPolicy.State.ACTIVE,p.state)
        assertFalse(p.expired(20000))
        assertTrue(p.downgrade())
        assertFalse(p.requiresHardware())
    }
}

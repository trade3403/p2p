package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class NetworkChangePolicyTest {
    @Test fun initialAndDuplicateNotificationsDoNotWakeConnections() {
        val p=NetworkChangePolicy()
        val wifi=NetworkChangePolicy.State(1L,listOf("a"))
        assertNull(p.observe(wifi))
        assertNull(p.observe(wifi))
        assertEquals(NetworkChangePolicy.Change(false),p.observe(NetworkChangePolicy.State(null,emptyList())))
        assertEquals(NetworkChangePolicy.Change(true),p.observe(wifi))
        assertNull(p.observe(wifi))
        assertEquals(NetworkChangePolicy.Change(true),p.observe(wifi.copy(addresses=listOf("b"))))
        assertEquals(NetworkChangePolicy.Change(true),p.observe(wifi.copy(handle=2L)))
        p.reset()
        assertNull(p.observe(wifi))
    }
}

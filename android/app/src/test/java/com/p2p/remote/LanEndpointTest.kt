package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LanEndpointTest {
    @Test fun acceptsOnlyLiteralPrivateIpv4() {
        assertEquals("ws://192.168.1.20:43821/direct", LanEndpoint.url("192.168.1.20"))
        assertEquals("ws://10.0.0.5:43821/direct", LanEndpoint.url("10.0.0.5"))
        assertEquals("ws://172.31.2.2:43821/direct", LanEndpoint.url("172.31.2.2"))
        for (host in listOf(
            "", "192.168.1.0", "192.168.1.255",
            "172.32.0.1", "8.8.8.8", "100.64.0.1",
            "example.com", "192.168.1.8:43821", "192.168.01.20",
            "192.168.1.20/../../", "192.168.999.1", "::1"
        )) {
            assertNull("should reject $host", LanEndpoint.url(host))
        }
    }
}

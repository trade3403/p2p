package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LanEndpointTest {
    @Test fun ranksOnlyReachableSubnetCandidatesWithoutProbing() {
        val subnets = listOf(LanEndpoint.LocalSubnet("192.168.4.15",24),
            LanEndpoint.LocalSubnet("10.1.2.3",16), LanEndpoint.LocalSubnet("192.168.4.15",28))
        assertEquals(listOf("192.168.4.12","192.168.4.44","10.1.20.5"),
            LanEndpoint.rankedCandidates(listOf("172.16.1.3","10.1.20.5","192.168.4.44",
                "192.168.4.12","192.168.4.12","192.168.4.15","8.8.8.8"),subnets))
    }

    @Test fun rejectsSubnetNetworkBroadcastAndNonPrivateInterfaces() {
        val subnets = listOf(LanEndpoint.LocalSubnet("192.168.2.67",26),
            LanEndpoint.LocalSubnet("8.8.8.9",24))
        assertEquals(listOf("192.168.2.68"),
            LanEndpoint.rankedCandidates(listOf("192.168.2.64","192.168.2.127",
                "192.168.2.68","192.168.2.67","192.168.2.130","8.8.8.10"),subnets))
        assertEquals(emptyList<String>(),LanEndpoint.rankedCandidates(listOf("10.1.2.3"),emptyList()))
    }

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

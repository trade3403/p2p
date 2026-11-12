package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test

class UdpPunchProtocolTest {
    @Test fun aesChallengeRejectsTamperingAndWrongKey() {
        val key = ByteArray(32) { it.toByte() }
        val body = ByteArray(16) { (42 + it).toByte() }
        val packet = UdpPunchProtocol.encrypt(key,UdpPunchProtocol.HELLO,body)
        assertEquals(45,packet.size)
        val decoded = UdpPunchProtocol.decrypt(key,packet)
        assertNotNull(decoded)
        assertEquals(UdpPunchProtocol.HELLO,decoded!!.first)
        assertArrayEquals(body,decoded.second)
        val mutated = packet.clone()
        mutated[0] = UdpPunchProtocol.ACK.toByte()
        assertNull(UdpPunchProtocol.decrypt(key,mutated))
        assertNull(UdpPunchProtocol.decrypt(ByteArray(32),packet))
        assertNull(UdpPunchProtocol.decrypt(key,packet.copyOfRange(0,30)))
    }

    @Test fun boundedPublicCandidateValidation() {
        val check = { address: String, role: String ->
            UdpPunchProtocol.parseCandidateFields(
                1, "candidate", "host", address, 49990, role
            )
        }
        assertEquals(49990,check("203.0.113.23","host")!!.port)
        assertNull(check("203.0.113.23","viewer"))
        for (bad in listOf("127.0.0.1","224.3.4.5","0.1.2.3",
            "test.example","169.254.0.3")) {
            assertNull("accepted $bad",check(bad,"host"))
        }
        assertNull(UdpPunchProtocol.parseCandidateFields(
            1,"candidate","host","203.0.113.23",0,"host"
        ))
        assertNull(UdpPunchProtocol.parseCandidateFields(
            2,"candidate","host","203.0.113.23",49990,"host"
        ))
    }
}

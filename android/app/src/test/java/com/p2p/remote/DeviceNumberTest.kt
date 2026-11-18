package com.p2p.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class DeviceNumberTest {
    @Test fun matchesWindowsDomainSeparatedNineDigitNumber() {
        val a = ByteArray(32) { 0x52.toByte() }
        val b = ByteArray(32) { 0x53.toByte() }
        assertEquals("822915239", DeviceNumber.fromSecret(a))
        assertEquals(9, DeviceNumber.fromSecret(a).length)
        assertNotEquals(DeviceNumber.fromSecret(a), DeviceNumber.fromSecret(b))
    }

    @Test(expected = IllegalArgumentException::class)
    fun rejectsShortPairingKey() {
        DeviceNumber.fromSecret(ByteArray(4))
    }
}

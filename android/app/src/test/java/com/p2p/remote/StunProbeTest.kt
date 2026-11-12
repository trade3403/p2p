package com.p2p.remote

import org.junit.Assert.*
import org.junit.Test
import java.net.InetAddress

class StunProbeTest {
    private fun reply(req:ByteArray, ip:IntArray,port:Int):ByteArray {
        val b=ByteArray(32)
        b[0]=1;b[1]=1;b[3]=12
        b[4]=0x21;b[5]=0x12;b[6]=0xA4.toByte();b[7]=0x42
        System.arraycopy(req,8,b,8,12)
        b[21]=0x20;b[23]=8;b[25]=1
        val p=port xor 0x2112
        b[26]=(p ushr 8).toByte();b[27]=p.toByte()
        val magic=intArrayOf(0x21,0x12,0xA4,0x42)
        for(i in 0..3)b[28+i]=(ip[i] xor magic[i]).toByte()
        return b
    }
    @Test fun parsesXorMappedAddress() {
        val req=StunProbe.request()
        val raw=reply(req,intArrayOf(203,0,113,8),42531)
        val parsed=StunProbe.parseResponse(raw,req)
        assertNotNull(parsed)
        assertEquals("203.0.113.8",parsed!!.address.hostAddress)
        assertEquals(42531,parsed.port)
        raw[8]=(raw[8].toInt() xor 1).toByte()
        assertNull(StunProbe.parseResponse(raw,req))
    }
    @Test fun rejectsMalformations() {
        val req=StunProbe.request()
        val response=reply(req,intArrayOf(198,51,100,10),4432)
        assertNull(StunProbe.parseResponse(response.copyOfRange(0,25),req))
        response[20]=0;response[21]=1
        assertNull(StunProbe.parseResponse(response,req))
        assertNull(StunProbe.parseResponse(ByteArray(0),req))
        assertNull(StunProbe.parseResponse(response,ByteArray(19)))
    }
}

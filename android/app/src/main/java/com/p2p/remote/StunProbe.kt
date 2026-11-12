package com.p2p.remote

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.URI
import java.security.SecureRandom

/**
 * RFC8489 STUN Binding diagnostics. Reports a one-shot NAT mapping only:
 * the returned port is NOT a live ICE candidate (socket closes afterwards).
 * Never sends relay credentials, AES keys, video, or clipboard contents.
 */
internal object StunProbe {
    const val PORT = 3478
    private val cookie = byteArrayOf(0x21, 0x12, 0xA4.toByte(), 0x42)

    data class Observation(val address: InetSocketAddress, val millis: Long)

    fun request(): ByteArray = ByteArray(20).apply {
        this[1] = 1
        System.arraycopy(cookie,0,this,4,4)
        val random = ByteArray(12).also { SecureRandom().nextBytes(it) }
        System.arraycopy(random,0,this,8,12)
    }

    fun parseResponse(response: ByteArray, req: ByteArray): InetSocketAddress? {
        if (req.size != 20 || response.size !in 32..512) return null
        fun at(index:Int): Int = response[index].toInt() and 255
        if (at(0) != 1 || at(1) != 1 || response.sliceArray(4..7).contentEquals(cookie).not() ||
            !response.sliceArray(8..19).contentEquals(req.sliceArray(8..19))) return null
        val length = at(2)*256 + at(3)
        if (length % 4 != 0 || length + 20 != response.size) return null
        var offset = 20
        while (offset + 4 <= response.size) {
            val attrType = at(offset)*256 + at(offset+1)
            val attrLength = at(offset+2)*256 + at(offset+3)
            val start = offset+4
            if (attrLength > response.size-start) return null
            if (attrType == 0x0020 && attrLength == 8 && at(start+1) == 1) {
                val encodedPort = at(start+2)*256 + at(start+3)
                val port = encodedPort xor 0x2112
                if (port == 0) return null
                val ip = ByteArray(4) { index -> (at(start+4+index) xor (cookie[index].toInt() and 255)).toByte() }
                return InetSocketAddress(InetAddress.getByAddress(ip),port)
            }
            offset = start + ((attrLength+3) and -4)
        }
        return null
    }

    fun probe(relayUrl: String): Observation {
        val parsed = URI(relayUrl.trim())
        require(parsed.scheme == "wss" || parsed.scheme == "https") { "需要 HTTPS/WSS 中转地址" }
        require(parsed.rawUserInfo == null && parsed.rawQuery == null && parsed.rawFragment == null)
        val host = requireNotNull(parsed.host) { "缺少中转域名" }
        val addr = InetAddress.getAllByName(host).firstOrNull { it is Inet4Address }
            ?: throw IllegalArgumentException("STUN 服务器缺少 IPv4 地址")
        val req = request()
        DatagramSocket().use { socket ->
            socket.soTimeout = 2000
            socket.connect(addr, PORT)
            val started = System.nanoTime()
            socket.send(DatagramPacket(req,req.size))
            val response = DatagramPacket(ByteArray(512),512)
            socket.receive(response)
            val external = parseResponse(response.data.copyOf(response.length),req)
                ?: error("STUN 回复无效或事务 ID 不匹配")
            return Observation(external,(System.nanoTime()-started)/1_000_000L)
        }
    }
}

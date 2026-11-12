package com.p2p.remote

import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import kotlin.concurrent.thread

/**
 * V0.9 user-initiated WAN UDP connectivity proof.
 *
 * The same bound DatagramSocket runs STUN and sends encrypted challenge/acks.
 * Existing HTTPS/WSS relay continues to carry all video and remote input.
 * Never put a bearer token, AES key, clipboard or desktop content in UDP.
 */
internal class UdpPunchProbe(
    private val relayUrl: String,
    secret: ByteArray,
    private val signal: (String) -> Unit,
    private val updateStatus: (String) -> Unit,
) {
    private val key = secret.copyOf()
    private val closed = AtomicBoolean(false)
    private val challenge = ByteArray(16).also { SecureRandom().nextBytes(it) }

    @Volatile private var socket: DatagramSocket? = null
    @Volatile private var remote: InetSocketAddress? = null

    fun setCandidate(json: JSONObject) {
        UdpPunchProtocol.parseCandidate(json, "host")?.let { remote = it }
    }

    fun start() {
        thread(name = "P2P-UDP-V09", isDaemon = true) {
            try {
                DatagramSocket().use { udp ->
                    socket = udp
                    val observation = StunProbe.probeOnSocket(udp, relayUrl)
                    if (closed.get()) return@thread
                    val candidateJson = JSONObject()
                        .put("v", 1)
                        .put("type", "candidate")
                        .put("role", "viewer")
                        .put("ip", observation.address.address.hostAddress)
                        .put("port", observation.address.port)
                        .toString()
                    signal(candidateJson)
                    updateStatus("UDP 本地映射已发现，等待电脑候选地址")
                    udp.soTimeout = 180
                    val started = System.nanoTime()
                    var lastSent = 0L
                    var lastCandidate = System.nanoTime()
                    var confirmedAt = 0L
                    val buffer = ByteArray(256)
                    while (!closed.get() && System.nanoTime() - started < 12_000_000_000L) {
                        if (confirmedAt != 0L && System.nanoTime() - confirmedAt > 1_000_000_000L) {
                            return@thread // Let the peer receive final ACK before closing.
                        }
                        val peer = remote
                        val now = System.nanoTime()
                        if (peer == null && now - lastCandidate >= 1_500_000_000L) {
                            // The WSS relay may drop candidate messages if its
                            // bounded queue is saturated by video frames.
                            signal(candidateJson)
                            lastCandidate = now
                        }
                        if (peer != null && now - lastSent >= 300_000_000L) {
                            val hello = UdpPunchProtocol.encrypt(key, UdpPunchProtocol.HELLO, challenge)
                            udp.send(DatagramPacket(hello, hello.size, peer))
                            lastSent = now
                        }
                        try {
                            val response = DatagramPacket(buffer, buffer.size)
                            udp.receive(response)
                            if (peer == null || response.address != peer.address ||
                                response.port != peer.port) continue
                            val packet = UdpPunchProtocol.decrypt(
                                key, response.data.copyOfRange(0, response.length)
                            ) ?: continue
                            if (packet.first == UdpPunchProtocol.HELLO && packet.second.size == 16) {
                                val ack = UdpPunchProtocol.encrypt(
                                    key, UdpPunchProtocol.ACK, packet.second
                                )
                                udp.send(DatagramPacket(ack, ack.size, peer))
                            } else if (packet.first == UdpPunchProtocol.ACK &&
                                packet.second.contentEquals(challenge) && confirmedAt == 0L) {
                                confirmedAt = System.nanoTime()
                                updateStatus("UDP 双端加密握手成功（仅验证连通性），画面仍通过 VPS")
                            }
                        } catch (_: java.net.SocketTimeoutException) {
                            // Normal bounded polling interval.
                        } catch (error: java.net.SocketException) {
                            if (!closed.get()) throw error
                            return@thread
                        }
                    }
                    if (!closed.get() && confirmedAt == 0L) {
                        updateStatus("UDP 双端探测超时，可能受对称 NAT 或防火墙限制；保留 VPS")
                    }
                }
            } catch (_: Exception) {
                if (!closed.get()) updateStatus("UDP 穿透验证不可用；请确认 VPS UDP 3478 已启用")
            } finally {
                socket = null
                key.fill(0)
            }
        }
    }

    fun stop() {
        closed.set(true)
        socket?.close()
    }
}

/** AES-256-GCM format interoperable with the Windows Rust protocol module. */
internal object UdpPunchProtocol {
    const val HELLO = 11
    const val ACK = 12

    fun parseCandidate(json: JSONObject, expectedRole: String): InetSocketAddress? =
        parseCandidateFields(
            json.optInt("v"), json.optString("type"), json.optString("role"),
            json.optString("ip"), json.optInt("port"), expectedRole
        )

    // Pure JVM-testable validation. Never resolves names from UDP candidates.
    fun parseCandidateFields(
        version: Int, type: String, role: String, address: String,
        port: Int, expectedRole: String
    ): InetSocketAddress? {
        if (version != 1 || type != "candidate" || role != expectedRole) return null
        val components = address.split(".")
        if (components.size != 4 || components.any {
                it.isEmpty() || it.length > 3 ||
                    (it.length > 1 && it[0] == '0') ||
                    it.any { ch -> ch !in '0'..'9' }
            }) return null
        val numbers = components.map { it.toIntOrNull() ?: return null }
        if (numbers.any { it !in 0..255 } ||
            numbers[0] == 0 || numbers[0] == 127 ||
            numbers[0] == 169 || numbers[0] >= 224) return null
        if (port !in 1..65535) return null
        val ip = InetAddress.getByAddress(numbers.map { it.toByte() }.toByteArray())
        return InetSocketAddress(ip,port)
    }

    fun encrypt(secret: ByteArray, kind: Int, body: ByteArray): ByteArray {
        require(secret.size == 32)
        require(body.size == 16)
        require(kind == HELLO || kind == ACK)
        val nonce = ByteArray(12).also { SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(secret,"AES"),GCMParameterSpec(128,nonce))
        cipher.updateAAD(byteArrayOf(kind.toByte()))
        return byteArrayOf(kind.toByte()) + nonce + cipher.doFinal(body)
    }

    fun decrypt(secret: ByteArray, packet: ByteArray): Pair<Int, ByteArray>? {
        if (secret.size != 32 || packet.size != 45) return null
        val kind = packet[0].toInt() and 255
        if (kind != HELLO && kind != ACK) return null
        return try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(secret,"AES"),
                GCMParameterSpec(128,packet.copyOfRange(1,13)))
            cipher.updateAAD(packet.copyOfRange(0,1))
            val message = cipher.doFinal(packet.copyOfRange(13,packet.size))
            if (message.size == 16) Pair(kind,message) else null
        } catch (_: Exception) {
            null
        }
    }
}

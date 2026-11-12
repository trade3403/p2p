package com.p2p.remote

import java.nio.ByteBuffer
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Bounded, single-inflight encrypted JPEG reassembler for experimental UDP
 * preview. Never accepts plaintext or frames from an unverified peer.
 *
 * Wire plaintext: frameId:u32 BE | parts:u16 BE | index:u16 BE |
 * jpegBytes:u32 BE | chunk (max 900 bytes).
 * Wire encrypted: kind:14 | 12-byte GCM nonce | cipher+16-byte tag.
 */
internal class UdpJpegAssembler {
    private val maxFrame = 96 * 1024
    private val chunkSize = 900
    private val maxChunks = 110
    private var assemblingId = 0L
    private var completedId = 0L
    var lastCompletedFrameId: Long = 0L
        private set
    private var startedMs = 0L
    private var expectedParts = 0
    private var expectedBytes = 0
    private var receivedParts = 0
    private var receivedBytes = 0
    private var fragments: Array<ByteArray?> = emptyArray()

    fun clear() {
        assemblingId = 0
        completedId = 0
        lastCompletedFrameId = 0
        expectedParts = 0
        expectedBytes = 0
        receivedParts = 0
        receivedBytes = 0
        fragments = emptyArray()
    }

    fun accept(packet: ByteArray, secret: ByteArray, nowMs: Long): ByteArray? {
        if (secret.size != 32 || packet.size !in 42..941 ||
            (packet[0].toInt() and 255) != 14) return null
        val plaintext = try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.DECRYPT_MODE,
                SecretKeySpec(secret, "AES"),
                GCMParameterSpec(128, packet.copyOfRange(1, 13))
            )
            cipher.updateAAD(byteArrayOf(14))
            cipher.doFinal(packet.copyOfRange(13, packet.size))
        } catch (_: Exception) { return null }
        if (plaintext.size !in 13..912) return null
        val header = ByteBuffer.wrap(plaintext)
        val id = header.int.toLong() and 0xffffffffL
        val count = header.short.toInt() and 0xffff
        val index = header.short.toInt() and 0xffff
        val size = header.int
        if (id == 0L || id <= completedId || size !in 4..maxFrame ||
            count !in 1..maxChunks || index >= count ||
            count != (size + chunkSize - 1) / chunkSize) return null
        val start = index * chunkSize
        val expectedChunk = minOf(chunkSize, size - start)
        if (expectedChunk < 1 || plaintext.size - 12 != expectedChunk) return null

        if (assemblingId != id) {
            if (id < assemblingId) return null
            assemblingId = id
            startedMs = nowMs
            expectedParts = count
            expectedBytes = size
            receivedParts = 0
            receivedBytes = 0
            fragments = arrayOfNulls(count)
        }
        if (nowMs - startedMs > 1500L || expectedParts != count ||
            expectedBytes != size) return null
        if (fragments[index] == null) {
            val bytes = plaintext.copyOfRange(12, plaintext.size)
            fragments[index] = bytes
            receivedParts++
            receivedBytes += bytes.size
        }
        if (receivedParts != count || receivedBytes != size) return null

        val image = ByteArray(size)
        var position = 0
        for (fragment in fragments) {
            val value = fragment ?: return null
            value.copyInto(image, position)
            position += value.size
        }
        if (image[0] != 0xff.toByte() || image[1] != 0xd8.toByte() ||
            image[size-2] != 0xff.toByte() || image[size-1] != 0xd9.toByte()) {
            return null
        }
        completedId = id
        lastCompletedFrameId = id
        assemblingId = 0L
        fragments = emptyArray()
        receivedParts = 0
        receivedBytes = 0
        return image
    }
}

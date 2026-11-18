package com.p2p.remote

import java.nio.ByteBuffer
import java.security.MessageDigest

/** Display-only 9-digit ID matching Windows; never an authentication secret. */
internal object DeviceNumber {
    fun fromSecret(secret: ByteArray): String {
        require(secret.size == 32)
        val domain = "p2p-device-number-v1:".toByteArray(Charsets.UTF_8)
        val digest = MessageDigest.getInstance("SHA-256").digest(domain + secret)
        val bits = ByteBuffer.wrap(digest, 0, 8).long
        return java.lang.Long.remainderUnsigned(bits, 1_000_000_000L)
            .toString().padStart(9, '0')
    }
}

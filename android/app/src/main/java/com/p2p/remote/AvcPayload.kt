package com.p2p.remote

/** V0.6: [u16 width][u16 height][Annex-B AVC access unit]. */
internal data class AvcPayload(val width: Int, val height: Int, val bytes: ByteArray) {
    companion object {
        fun parse(packet: ByteArray): AvcPayload? {
            if (packet.size !in 9..(2 * 1024 * 1024)) return null
            val width = ((packet[0].toInt() and 255) shl 8) or (packet[1].toInt() and 255)
            val height = ((packet[2].toInt() and 255) shl 8) or (packet[3].toInt() and 255)
            if (width !in 2..1920 || height !in 2..1080 || width % 2 != 0 || height % 2 != 0) return null
            val annexB3 = packet[4] == 0.toByte() && packet[5] == 0.toByte() && packet[6] == 1.toByte()
            val annexB4 = packet[4] == 0.toByte() && packet[5] == 0.toByte() &&
                packet[6] == 0.toByte() && packet[7] == 1.toByte()
            if (!annexB3 && !annexB4) return null
            return AvcPayload(width, height, packet)
        }
    }
}

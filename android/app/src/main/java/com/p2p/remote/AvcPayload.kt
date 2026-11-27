package com.p2p.remote

/** V0.6: [u16 width][u16 height][Annex-B AVC access unit]. */
internal data class AvcPayload(val width: Int, val height: Int, val bytes: ByteArray) {
    // Structural random-access check, not a substitute for MediaCodec decode
    // or parsing the referenced SPS/PPS IDs. Never restart on a bare IDR.
    fun hasSelfContainedIdr(): Boolean {
        fun prefixLength(i: Int): Int {
            if (i + 2 >= bytes.size || bytes[i] != 0.toByte() || bytes[i+1] != 0.toByte()) return 0
            if (bytes[i+2] == 1.toByte()) return 3
            if (i + 3 < bytes.size && bytes[i+2] == 0.toByte() && bytes[i+3] == 1.toByte()) return 4
            return 0
        }
        var pos = 4
        var sps = false
        var pps = false
        while (pos < bytes.size) {
            val prefix = prefixLength(pos)
            if (prefix == 0) return false
            val header = pos + prefix
            if (header >= bytes.size) return false
            val nal = bytes[header].toInt() and 255
            if (nal and 128 != 0 || (nal and 31) !in 1..23) return false
            var next = header + 1
            while (next < bytes.size && prefixLength(next) == 0) next++
            var end = next
            // Annex-B trailing_zero_8bits are not NAL payload bytes.
            while (end > header + 1 && bytes[end-1] == 0.toByte()) end--
            if (end <= header + 1) return false
            when (nal and 31) {
                7 -> sps = true
                8 -> pps = true
                5 -> return sps && pps
                in 1..4 -> return false
            }
            pos = next
        }
        return false
    }

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

package com.p2p.remote

/**
 * LAN direct is deliberately limited to literal RFC1918 IPv4 hosts. Never
 * resolve arbitrary DNS names or send the VPS bearer token over a ws:// link.
 */
internal object LanEndpoint {
    const val PORT = 43821

    fun isOnLocalSubnet(candidate: String): Boolean {
        if (url(candidate) == null) return false
        val target = candidate.trim().split(".").map { it.toInt() }
            .fold(0L) { acc, octet -> (acc shl 8) or octet.toLong() }
        return runCatching {
            val interfaces = java.net.NetworkInterface.getNetworkInterfaces() ?: return false
            java.util.Collections.list(interfaces).any { nic ->
                nic.isUp && !nic.isLoopback && nic.interfaceAddresses.any { entry ->
                    val address = entry.address as? java.net.Inet4Address
                    val prefix = entry.networkPrefixLength.toInt()
                    if (address == null || prefix !in 1..30 || url(address.hostAddress.orEmpty()) == null) false
                    else {
                        val local = address.address.fold(0L) { acc, octet ->
                            (acc shl 8) or (octet.toInt() and 255).toLong()
                        }
                        local != target && (local ushr (32 - prefix)) == (target ushr (32 - prefix))
                    }
                }
            }
        }.getOrDefault(false)
    }

    fun url(ipAddress: String): String? {
        val octets = ipAddress.trim().split(".")
        if (octets.size != 4 || octets.any {
                it.isEmpty() || it.length > 3 || (it.length > 1 && it.startsWith("0")) ||
                    it.any { char -> char !in '0'..'9' }
            }) return null
        val addr = octets.map { it.toIntOrNull() ?: return null }
        if (addr.any { it !in 0..255 }) return null
        val allowed =
            addr[0] == 10 ||
                (addr[0] == 172 && addr[1] in 16..31) ||
                (addr[0] == 192 && addr[1] == 168)
        if (!allowed || addr[3] == 0 || addr[3] == 255) return null
        return "ws://${addr.joinToString(".")}:$PORT/direct"
    }
}

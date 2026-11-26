package com.p2p.remote

/**
 * LAN direct is deliberately limited to literal RFC1918 IPv4 hosts. Never
 * resolve arbitrary DNS names or send the VPS bearer token over a ws:// link.
 */
internal object LanEndpoint {
    const val PORT = 43821

    internal data class LocalSubnet(val address: String, val prefix: Int)

    private fun number(ip: String): Long? {
        if (url(ip) == null) return null
        return ip.trim().split(".").fold(0L) { acc, octet -> (acc shl 8) or octet.toLong() }
    }

    private fun localSubnets(): List<LocalSubnet> = runCatching {
        val interfaces = java.net.NetworkInterface.getNetworkInterfaces() ?: return emptyList()
        java.util.Collections.list(interfaces).filter { it.isUp && !it.isLoopback }
            .flatMap { nic -> nic.interfaceAddresses.mapNotNull { entry ->
                val address = entry.address as? java.net.Inet4Address ?: return@mapNotNull null
                LocalSubnet(address.hostAddress.orEmpty(), entry.networkPrefixLength.toInt())
            } }
    }.getOrDefault(emptyList())

    // Only advertised literals on a live local RFC1918 subnet are eligible.
    // Prefer longest matching prefix, then preserve the host's order.
    internal fun rankedCandidates(candidates: List<String>, subnets: List<LocalSubnet>): List<String> {
        val local = subnets.mapNotNull { subnet ->
            if (subnet.prefix !in 1..30) null
            else number(subnet.address)?.let { it to subnet.prefix }
        }
        val own = subnets.mapNotNull { number(it.address) }.toSet()
        return candidates.take(17).map { it.trim() }.distinct().mapNotNull { candidate ->
            val target = number(candidate) ?: return@mapNotNull null
            if (target in own) return@mapNotNull null
            val prefix = local.mapNotNull { (ip, prefix) ->
                val hostMask = (1L shl (32-prefix))-1
                val network = ip and hostMask.inv()
                if ((target and hostMask.inv()) == network &&
                    target != network && target != (network or hostMask)) prefix else null
            }.maxOrNull() ?: return@mapNotNull null
            candidate to prefix
        }.sortedByDescending { it.second }.map { it.first }
    }

    fun selectAdvertised(candidates: List<String>): String? =
        rankedCandidates(candidates, localSubnets()).firstOrNull()

    fun isOnLocalSubnet(candidate: String): Boolean =
        selectAdvertised(listOf(candidate)) != null

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

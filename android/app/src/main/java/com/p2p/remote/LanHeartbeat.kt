package com.p2p.remote

/** Authenticated LAN PING/PONG lifetime; one outstanding challenge. */
internal class LanHeartbeat(private val nonce: () -> Long) {
    data class Ping(val token: Long, val sentAt: Long)
    private var nextAt = 0L
    private var pending: Ping? = null

    fun reset(now: Long) {
        pending = null
        nextAt = now
    }

    fun expired(now: Long): Boolean =
        pending?.let { now - it.sentAt >= TIMEOUT_MS } ?: false

    fun poll(now: Long): Ping? {
        if (pending != null || now < nextAt) return null
        return Ping(nonce(), now).also { pending = it }
    }

    fun pong(token: Long, now: Long): Long? {
        val ping = pending ?: return null
        if (ping.token != token || expired(now)) return null
        pending = null
        nextAt = now + INTERVAL_MS
        return (now - ping.sentAt).coerceAtLeast(0L)
    }

    companion object {
        const val INTERVAL_MS = 4000L
        const val TIMEOUT_MS = 12000L
    }
}

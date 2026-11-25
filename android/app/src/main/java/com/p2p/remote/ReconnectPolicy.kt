package com.p2p.remote

/** Deliberately bounded retries; user must explicitly start the session. */
internal object ReconnectPolicy {
    fun delayMs(attempt: Int): Long {
        val exponent = attempt.coerceIn(0, 5)
        return (1L shl exponent).coerceAtMost(30L) * 1000L
    }
}

/** One LAN attempt per user-requested session; reconnects stay on WSS. */
internal class LanAttemptPolicy {
    var attempted = false
        private set

    fun reset() { attempted = false }

    fun take(endpoint: String): Boolean {
        if (attempted || endpoint.isEmpty()) return false
        attempted = true
        return true
    }

    fun canFallback(token: String): Boolean = attempted && token.length >= 32

    companion object {
        const val CONNECT_TIMEOUT_MS = 4000L
        fun canOpen(requested: Boolean, socketPresent: Boolean): Boolean =
            requested && !socketPresent
    }
}

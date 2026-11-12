package com.p2p.remote

/** Deliberately bounded retries; user must explicitly start the session. */
internal object ReconnectPolicy {
    fun delayMs(attempt: Int): Long {
        val exponent = attempt.coerceIn(0, 5)
        return (1L shl exponent).coerceAtMost(30L) * 1000L
    }
}

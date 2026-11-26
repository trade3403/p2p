package com.p2p.remote

/**
 * Main-thread, per-user-connect diagnostic state. All timestamps are from
 * elapsedRealtime, never wall clock. A relay WebSocket opening is not proof
 * that the remote desktop has recovered. No URLs, IPs or exception text.
 */
internal class LanRecoveryMetrics {
    var attemptMs: Long? = null
        private set
    var reason: String? = null
        private set
    var fallbackStarted: Boolean = false
        private set
    var relayOpenMs: Long? = null
        private set
    var firstVideoMs: Long? = null
        private set
    private var attemptAt: Long? = null
    private var failedAt: Long? = null

    fun reset() {
        attemptAt = null
        failedAt = null
        attemptMs = null
        reason = null
        fallbackStarted = false
        relayOpenMs = null
        firstVideoMs = null
    }

    fun beginLan(now: Long) {
        attemptAt = now
    }

    fun verified(now: Long) {
        attemptAt?.let { attemptMs = (now - it).coerceAtLeast(0L) }
        attemptAt = null
    }

    fun failed(code: String, canFallback: Boolean, now: Long) {
        if (failedAt != null) return
        verified(now)
        reason = code
        fallbackStarted = canFallback
        failedAt = now
    }

    fun relayOpened(now: Long) {
        if (!fallbackStarted || relayOpenMs != null) return
        failedAt?.let { relayOpenMs = (now - it).coerceAtLeast(0L) }
    }

    fun relayVideo(now: Long) {
        if (!fallbackStarted || relayOpenMs == null || firstVideoMs != null) return
        failedAt?.let { firstVideoMs = (now - it).coerceAtLeast(0L) }
    }
}

package com.p2p.remote

/**
 * One bounded batch per user connect. All times are elapsedRealtime.
 * No candidate may be revisited, even after an authenticated LAN disconnect.
 * The legacy native Activity retains its original single-attempt policy.
 */
internal class LanCandidatePlan {
    data class Attempt(val address: String, val deadlineMs: Long)
    var attempted = false
        private set
    var attemptCount = 0
        private set
    private var remaining = emptyList<String>()
    private var deadlineMs = 0L
    private var finished = false

    fun reset() {
        attempted = false
        attemptCount = 0
        remaining = emptyList()
        deadlineMs = 0L
        finished = false
    }

    fun begin(addresses: List<String>, now: Long) {
        if (attempted || addresses.isEmpty()) return
        attempted = true
        remaining = addresses.take(17).distinct()
        deadlineMs = now + TOTAL_TIMEOUT_MS
    }

    fun take(now: Long, eligible: (String) -> Boolean): Attempt? {
        if (!attempted || finished || attemptCount >= MAX_ATTEMPTS || now >= deadlineMs) {
            remaining = emptyList()
            return null
        }
        while (remaining.isNotEmpty()) {
            val address = remaining.first()
            remaining = remaining.drop(1)
            if (!eligible(address)) continue
            attemptCount++
            return Attempt(address, minOf(deadlineMs, now + PER_ATTEMPT_TIMEOUT_MS))
        }
        return null
    }

    fun verified() {
        finished = true
        remaining = emptyList()
    }

    fun canFallback(token: String): Boolean = attempted && token.length >= 32

    companion object {
        const val MAX_ATTEMPTS = 3
        const val PER_ATTEMPT_TIMEOUT_MS = 4000L
        const val TOTAL_TIMEOUT_MS = 8000L
    }
}

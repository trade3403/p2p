package com.p2p.remote

/**
 * Main-thread recovery admission state, separate from the initial LAN batch.
 * Only current authenticated relay advertisements are eligible. A stable
 * candidate set must be observed over time; repeated metadata is not a retry.
 * The caller must reserve before starting any future parallel LAN handshake.
 */
internal class LanReadmissionPolicy {
    private var failed = false
    private var eligible = emptyList<String>()
    private var stableSince: Long? = null
    private var lastObservedAt: Long? = null
    private var nextAllowedAt = 0L
    private var inFlight = false
    private var generation = 0L
    var reservations = 0
        private set

    data class Reservation(val generation: Long, val candidates: List<String>)

    fun reset() {
        generation++
        failed = false
        eligible = emptyList()
        stableSince = null
        lastObservedAt = null
        nextAllowedAt = 0
        inFlight = false
        reservations = 0
    }

    fun lanFailed(now: Long) {
        generation++
        failed = true
        eligible = emptyList()
        stableSince = null
        lastObservedAt = null
        inFlight = false
        nextAllowedAt = now + delayMs(reservations)
    }

    // Network changes invalidate even an otherwise identical advertisement.
    // They do not bypass cooldown or renew the retry budget.
    fun networkChanged(now: Long) {
        if (inFlight) nextAllowedAt = maxOf(nextAllowedAt, now + delayMs(reservations))
        generation++
        eligible = emptyList()
        stableSince = null
        lastObservedAt = null
        inFlight = false
    }

    fun observe(now: Long, relayVideo: Boolean, candidates: List<String>) {
        val next = if (failed && relayVideo) candidates.take(17).distinct() else emptyList()
        val previous = lastObservedAt
        val expired = previous != null && now - previous > FRESH_MS
        if (next != eligible || expired) {
            if (inFlight) nextAllowedAt = maxOf(nextAllowedAt, now + delayMs(reservations))
            generation++
            inFlight = false
            eligible = next
            stableSince = if (next.isEmpty()) null else now
        }
        lastObservedAt = if (next.isEmpty()) null else now
    }

    fun state(now: Long): String = when {
        !failed -> "idle"
        inFlight -> "reserved"
        reservations >= MAX_BATCHES -> "budget_exhausted"
        eligible.isEmpty() || now - (lastObservedAt ?: 0L) > FRESH_MS -> "waiting_candidates"
        now < nextAllowedAt -> "cooldown"
        now - (stableSince ?: now) < STABLE_MS -> "stabilizing"
        else -> "eligible"
    }

    fun reserve(now: Long): Reservation? {
        if (state(now) != "eligible") return null
        generation++
        inFlight = true
        reservations++
        return Reservation(generation, eligible.toList())
    }

    // Reject stale completions after disconnect, network change or replacement.
    fun isCurrent(reservation: Reservation): Boolean =
        inFlight && generation == reservation.generation

    fun isFresh(reservation: Reservation, now: Long): Boolean =
        isCurrent(reservation) && lastObservedAt?.let { now - it in 0..FRESH_MS } == true

    fun complete(reservation: Reservation, now: Long, success: Boolean): Boolean {
        if (!isCurrent(reservation)) return false
        generation++
        inFlight = false
        eligible = emptyList()
        stableSince = null
        lastObservedAt = null
        failed = !success
        nextAllowedAt = now + delayMs(reservations)
        return true
    }

    companion object {
        const val STABLE_MS = 3000L
        const val FRESH_MS = 8000L
        const val MAX_BATCHES = 4
        fun delayMs(attempts: Int): Long =
            (10_000L shl attempts.coerceIn(0, 3)).coerceAtMost(80_000L)
    }
}

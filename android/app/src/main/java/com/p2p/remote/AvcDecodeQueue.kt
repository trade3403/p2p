package com.p2p.remote

/**
 * Admission state for a single ordered AVC worker. A loss serial belongs to
 * the next admitted access unit, not to older tasks already in the queue.
 * No Android dependencies: lifecycle and overload races are JVM-testable.
 */
internal class AvcDecodeQueue(private val capacity: Int = 2) {
    init { require(capacity > 0) }

    data class Ticket(val generation: Long, val lossSerial: Long)

    private var generation = 0L
    private var lossSerial = 0L
    private var pending = 0
    private var closed = false
    private var drops = 0L
    private var format: Pair<Int, Int>? = null

    @get:Synchronized
    val droppedFrames: Long get() = drops

    @Synchronized fun isCurrent(g: Long): Boolean = !closed && generation == g

    @Synchronized fun admit(width: Int = 0, height: Int = 0): Ticket? {
        if (closed) return null
        val nextFormat = width to height
        if (format != nextFormat) {
            // A new size retires old output callbacks and queued units even
            // if the old format filled the queue. First format after reset
            // already belongs to a fresh generation.
            if (format != null) invalidate()
            format = nextFormat
        }
        if (pending >= capacity) {
            drops++
            lossSerial++
            return null
        }
        pending++
        return Ticket(generation, lossSerial)
    }

    @Synchronized fun complete(ticket: Ticket) {
        if (isCurrent(ticket.generation)) {
            check(pending > 0) { "AVC admission completed twice" }
            pending--
        }
    }

    // Called for a MediaCodec input failure on the ordered worker. That
    // worker also immediately requires a new keyframe.
    @Synchronized fun recordWorkerDrop(g: Long) {
        if (isCurrent(g)) drops++
    }

    @Synchronized fun invalidate(): Boolean {
        if (closed) return false
        generation++
        pending = 0
        lossSerial = 0L
        format = null
        return true
    }

    @Synchronized fun close(): Boolean {
        if (closed) return false
        invalidate()
        closed = true
        return true
    }

    // Linearize the render submission with invalidation. The action must
    // be short and must not call back into the UI or wait for a handler.
    @Synchronized fun ifCurrent(g: Long, action: () -> Unit): Boolean {
        if (!isCurrent(g)) return false
        action()
        return true
    }
}

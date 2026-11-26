package com.p2p.remote

/** No network identifiers are exported to diagnostics or logs. */
internal class NetworkChangePolicy {
    data class State(val handle: Long?, val addresses: List<String>)
    data class Change(val available: Boolean)
    private var previous: State? = null

    fun reset() { previous = null }

    fun observe(state: State): Change? {
        val old = previous
        previous = state
        if (old == null || old == state) return null
        return Change(state.handle != null)
    }
}

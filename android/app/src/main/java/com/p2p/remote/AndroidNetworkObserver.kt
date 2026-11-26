package com.p2p.remote

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.os.Handler

/**
 * Default-network notifications are hints, not a guarantee of reachability.
 * Read the actual current state after a short debounce; never trust the
 * ordering of callbacks from old networks.
 */
internal class AndroidNetworkObserver(context: Context, private val main: Handler,
    private val onChange: (NetworkChangePolicy.Change) -> Unit) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val policy = NetworkChangePolicy()
    private var callback: ConnectivityManager.NetworkCallback? = null
    var ready = false
        private set
    private val sample = Runnable {
        if (callback != null) {
            readState()?.let {
                ready = true
                policy.observe(it)?.let(onChange)
            }
        }
    }

    private fun readState(): NetworkChangePolicy.State? = runCatching {
        val cm = requireNotNull(manager)
        val network = cm.activeNetwork
        val addresses = if (network == null) emptyList() else
            cm.getLinkProperties(network)?.linkAddresses?.map { it.toString() }?.sorted().orEmpty()
        NetworkChangePolicy.State(network?.networkHandle, addresses)
    }.getOrNull()

    fun start() {
        if (callback != null) return
        val cm = manager ?: return
        policy.reset()
        readState()?.let { policy.observe(it) }
        val listener = object : ConnectivityManager.NetworkCallback() {
            private fun signal() {
                main.post {
                    if (callback !== this) return@post
                    main.removeCallbacks(sample)
                    main.postDelayed(sample, 750L)
                }
            }
            override fun onAvailable(network: Network) = signal()
            override fun onLost(network: Network) = signal()
            override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = signal()
        }
        callback = listener
        try {
            cm.registerDefaultNetworkCallback(listener)
        } catch (_: Exception) {
            callback = null
        }
    }

    fun stop() {
        val listener = callback
        callback = null
        main.removeCallbacks(sample)
        if (listener != null) runCatching { manager?.unregisterNetworkCallback(listener) }
        policy.reset()
        ready = false
    }
}

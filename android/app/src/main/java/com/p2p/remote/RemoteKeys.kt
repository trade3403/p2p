package com.p2p.remote

/** Explicit list of keyboard operations supported by the Windows Host. */
internal object RemoteKeys {
    private val allowed = setOf(
        "enter", "backspace", "tab", "escape", "delete",
        "up", "down", "left", "right", "home", "end",
        "page_up", "page_down", "space",
        "ctrl_a", "ctrl_c", "ctrl_v", "ctrl_x", "ctrl_z", "ctrl_y"
    )

    fun isAllowed(name: String): Boolean = name in allowed
}

package com.p2p.remote

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteKeysTest {
    @Test fun navigationAndEditingKeysAllowed() {
        listOf(
            "up", "down", "left", "right", "home", "end",
            "page_up", "page_down", "enter", "backspace",
            "tab", "escape", "delete", "space",
            "ctrl_a", "ctrl_c", "ctrl_v", "ctrl_x", "ctrl_z", "ctrl_y"
        ).forEach { assertTrue(it, RemoteKeys.isAllowed(it)) }
    }

    @Test fun unexpectedPrivilegedOrMalformedKeysDenied() {
        listOf(
            "", "ctrl_alt_delete", "alt_f4", "win_r", "powershell",
            "CTRL_V", "ctrl_v\n", "up\u0000"
        ).forEach { assertFalse(it, RemoteKeys.isAllowed(it)) }
    }
}

//! Explicit, bounded remote operations; no background clipboard monitoring.
pub const MAX_CLIPBOARD_BYTES: usize = 4096;
pub const MAX_MONITORS: usize = 16;

pub fn valid_monitor_index(index: usize, actual_count: usize) -> bool {
    index < MAX_MONITORS && index < actual_count
}

pub fn valid_clipboard_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CLIPBOARD_BYTES
}


/// Only these bounded, explicit keyboard actions are accepted from a paired viewer.
/// Named commands, rather than arbitrary scan codes, prevent extending remote input
/// accidentally when the client and host run different protocol revisions.
pub fn valid_remote_key(value: &str) -> bool {
    matches!(value,
        "enter" | "backspace" | "tab" | "escape" | "delete"
        | "up" | "down" | "left" | "right"
        | "home" | "end" | "page_up" | "page_down" | "space"
        | "ctrl_a" | "ctrl_c" | "ctrl_v" | "ctrl_x" | "ctrl_z" | "ctrl_y"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_allowlist_rejects_unbounded_or_system_shortcuts() {
        for allowed in [
            "enter", "backspace", "tab", "escape", "delete",
            "up", "down", "left", "right", "home", "end",
            "page_up", "page_down", "space",
            "ctrl_a", "ctrl_c", "ctrl_v", "ctrl_x", "ctrl_z", "ctrl_y",
        ] {
            assert!(valid_remote_key(allowed), "expected {allowed}");
        }
        for rejected in [
            "", "cmd", "powershell", "ctrl_alt_delete",
            "win_r", "alt_f4", "f12", "CTRL_V", "left\u{0000}admin",
            "ctrl_v\n", "../exec",
        ] {
            assert!(!valid_remote_key(rejected), "unexpected {rejected:?}");
        }
    }
    #[test]
    fn monitor_selection_rejects_out_of_range_values() {
        assert!(valid_monitor_index(0, 2));
        assert!(valid_monitor_index(1, 2));
        assert!(!valid_monitor_index(2, 2));
        assert!(!valid_monitor_index(1, 0));
        assert!(!valid_monitor_index(16, 30));
    }
    #[test]
    fn clipboard_is_bounded_by_utf8_bytes_not_chars() {
        assert!(!valid_clipboard_text(""));
        assert!(valid_clipboard_text("hello"));
        assert!(valid_clipboard_text(&"文".repeat(1365)));
        assert!(!valid_clipboard_text(&"文".repeat(1366)));
    }
}

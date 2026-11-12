//! Explicit, bounded remote operations; no background clipboard monitoring.
pub const MAX_CLIPBOARD_BYTES: usize = 4096;
pub const MAX_MONITORS: usize = 16;

pub fn valid_monitor_index(index: usize, actual_count: usize) -> bool {
    index < MAX_MONITORS && index < actual_count
}

pub fn valid_clipboard_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CLIPBOARD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
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

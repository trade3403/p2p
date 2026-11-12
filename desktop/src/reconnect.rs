//! Conservative retry policy for a single personally authorized remote host.
use std::time::Duration;

pub fn retry_delay(failures: u32) -> Duration {
    let secs = (1_u64 << failures.min(5)).min(30);
    Duration::from_secs(secs)
}

pub fn authentication_rejected(error: &anyhow::Error) -> bool {
    if let Some(tokio_tungstenite::tungstenite::Error::Http(response)) =
        error.downcast_ref::<tokio_tungstenite::tungstenite::Error>()
    {
        return matches!(response.status().as_u16(), 401 | 403);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_are_bounded() {
        let expected = [1, 2, 4, 8, 16, 30, 30, 30];
        for (attempt, seconds) in expected.iter().enumerate() {
            assert_eq!(retry_delay(attempt as u32), Duration::from_secs(*seconds));
        }
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(30));
    }
}

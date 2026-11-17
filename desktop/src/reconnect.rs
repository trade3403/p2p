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

/// Map low-level connection failures to short Chinese UI hints.
pub fn describe_connect_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}").to_lowercase();
    if authentication_rejected(error) {
        return "中转访问口令错误（HTTP 401/403）".into();
    }
    if text.contains("timed out") || text.contains("timeout") {
        return "连接中转超时（检查网络、DNS 与安全组 443）".into();
    }
    if text.contains("dns")
        || text.contains("name or service not known")
        || text.contains("no such host")
        || text.contains("failed to lookup")
    {
        return "DNS 解析失败（确认域名是否指向 VPS）".into();
    }
    if text.contains("certificate")
        || text.contains("tls")
        || text.contains("ssl")
        || text.contains("handshake")
    {
        return "TLS 证书校验失败（确认 Caddy 证书与域名匹配）".into();
    }
    if text.contains("already connected") || text.contains("policy violation") {
        return "房间角色冲突（同 role 已有连接）".into();
    }
    if text.contains("public relay requires wss") {
        return "公网中转必须使用 wss://".into();
    }
    format!("中转连接失败: {error:#}")
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

    #[test]
    fn classifies_dns_and_tls_hints() {
        let dns = anyhow::anyhow!("failed to lookup address information: Name or service not known");
        assert!(describe_connect_error(&dns).contains("DNS"));
        let tls = anyhow::anyhow!("invalid peer certificate: UnknownIssuer");
        assert!(describe_connect_error(&tls).contains("TLS"));
        let timeout = anyhow::anyhow!("relay connection timed out");
        assert!(describe_connect_error(&timeout).contains("超时"));
    }
}

//! Explicit, process-wide rustls TLS crypto provider selection.
//!
//! Cargo feature unification may enable both aws-lc-rs and ring in rustls 0.23.
//! rustls cannot pick one automatically in that case, which previously caused
//! a panic in the Windows WSS host/viewer connection thread.
//! Initialize at program entry, before any TLS configuration or network thread.

pub fn initialize() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // If another thread installed a provider first, accepting that existing
        // process-wide provider is safer than panicking over the race.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_some(),
        "无法初始化 TLS 加密后端"
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_crypto_provider_allows_tls_client_config() {
        super::initialize();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        let _config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
    }
}

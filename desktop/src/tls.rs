//! Deterministic process-level TLS provider for rustls 0.23.
//!
//! Desktop dependencies may enable both aws-lc-rs and ring via additive Cargo
//! features. In that case rustls cannot infer a process-level provider and
//! panics from connect_async. Explicitly install ring before opening WSS.
//!
//! This function is safe to call from both UI initialization and worker threads.
//! An already installed provider (possibly installed by another library) is
//! accepted instead of attempting to override process-global crypto state.

pub fn initialize() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "A rustls CryptoProvider must be installed before a WSS connection"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_crypto_provider_is_installed_before_connect() {
        initialize();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        // Builds the same default rustls ClientConfig created by the WSS layer.
        // This regression test catches the runtime panic even if dependency
        // features are unified from several crates.
        let _client_builder = rustls::ClientConfig::builder();
        initialize(); // Re-entry is a no-op.
    }
}

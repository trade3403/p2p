//! Persist only the non-sensitive relay address, never a session secret or bearer token.
use std::{fs, path::PathBuf};
use serde::{Deserialize, Serialize};

const DEFAULT_RELAY: &str = "wss://remote.example.com";

#[derive(Serialize, Deserialize)]
struct PublicSettings {
    relay_url: String,
}

fn settings_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|base| PathBuf::from(base).join("P2PRemote").join("settings.json"))
}

fn is_safe_address(value: &str) -> bool {
    if value.len() > 512 { return false; }
    let Ok(url) = url::Url::parse(value) else { return false; };
    (url.scheme() == "wss" || url.scheme() == "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

pub fn load_relay_url() -> String {
    let Some(path) = settings_path() else { return DEFAULT_RELAY.to_owned(); };
    let Ok(bytes) = fs::read(path) else { return DEFAULT_RELAY.to_owned(); };
    let Ok(settings) = serde_json::from_slice::<PublicSettings>(&bytes) else {
        return DEFAULT_RELAY.to_owned();
    };
    if is_safe_address(&settings.relay_url) {
        settings.relay_url
    } else {
        DEFAULT_RELAY.to_owned()
    }
}

pub fn save_relay_url(value: &str) -> std::io::Result<()> {
    if !is_safe_address(value) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "relay must be a WSS/HTTPS URL without credentials"));
    }
    let Some(path) = settings_path() else { return Ok(()); };
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let bytes = serde_json::to_vec(&PublicSettings { relay_url: value.to_owned() })
        .map_err(std::io::Error::other)?;
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_nonsecret_relay_addresses() {
        assert!(is_safe_address("wss://relay.example.com"));
        assert!(is_safe_address("https://relay.example.com"));
        assert!(!is_safe_address("ws://relay.example.com"));
        assert!(!is_safe_address("wss://user:pass@relay.example.com"));
        assert!(!is_safe_address("wss://relay.example.com/?token=secret"));
    }
}

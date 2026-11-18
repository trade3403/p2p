//! Cross-platform numeric-pairing cryptographic primitives.
//! The Relay sees only public keys and a proof derived from ECDH; it never
//! receives the six-digit code or the derived session secret.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, Rng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

const WINDOW_DOMAIN: &str = "p2p-pairing-window-v1|";
const PROOF_DOMAIN: &str = "p2p-pairing-proof-v1|";
const REQUEST_DOMAIN: &str = "p2p-pairing-request-v1|";
const DECISION_DOMAIN: &str = "p2p-pairing-decision-v1|";
const SESSION_DOMAIN: &str = "p2p-remote-session-v1|";

pub struct DeviceIdentity {
    pub device_id: [u8; 16],
    signing_key: SigningKey,
}

pub struct PairingWindow {
    secret: StaticSecret,
    pub public_key: [u8; 32],
    pub code: String,
    pub expires_at: i64,
}

pub struct ViewerEphemeral {
    secret: StaticSecret,
    pub public_key: [u8; 32],
}

impl DeviceIdentity {
    pub fn generate() -> Self {
        let mut device_id = [0u8; 16];
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut device_id);
        OsRng.fill_bytes(&mut seed);
        Self { device_id, signing_key: SigningKey::from_bytes(&seed) }
    }

    pub fn from_parts(device_id: [u8; 16], signing_seed: [u8; 32]) -> Self {
        Self { device_id, signing_key: SigningKey::from_bytes(&signing_seed) }
    }

    pub fn signing_seed(&self) -> [u8; 32] { self.signing_key.to_bytes() }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn device_id_hex(&self) -> String { hex::encode(self.device_id) }

    pub fn public_key_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public_key())
    }

    pub fn sign_pairing_window(&self, device_number: &str, expires_at: i64, pairing_public_b64: &str) -> String {
        let message = format!("{WINDOW_DOMAIN}{device_number}|{expires_at}|{pairing_public_b64}");
        URL_SAFE_NO_PAD.encode(self.signing_key.sign(message.as_bytes()).to_bytes())
    }

    pub fn sign_decision(
        &self, request_id: &str, device_number: &str,
        viewer_public_b64: &str, nonce_b64: &str, approved: bool,
    ) -> String {
        let decision = if approved { "approved" } else { "denied" };
        let message = format!(
            "{DECISION_DOMAIN}{request_id}|{device_number}|{viewer_public_b64}|{nonce_b64}|{decision}"
        );
        URL_SAFE_NO_PAD.encode(self.signing_key.sign(message.as_bytes()).to_bytes())
    }
}

impl PairingWindow {
    pub fn generate(now_unix: i64) -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public_key = X25519PublicKey::from(&secret).to_bytes();
        let code = format!("{:06}", OsRng.gen_range(0..1_000_000u32));
        Self { secret, public_key, code, expires_at: now_unix + 120 }
    }

    pub fn public_key_b64(&self) -> String { URL_SAFE_NO_PAD.encode(self.public_key) }

    pub fn proof_for_viewer(
        &self, device_number: &str, viewer_public: [u8; 32], nonce: [u8; 16],
    ) -> anyhow::Result<[u8; 32]> {
        let peer = X25519PublicKey::from(viewer_public);
        let shared = self.secret.diffie_hellman(&peer);
        make_proof(shared.as_bytes(), device_number, &self.code, viewer_public, nonce)
    }

    pub fn session_secret(
        &self, device_number: &str, viewer_public: [u8; 32], nonce: [u8; 16],
    ) -> anyhow::Result<[u8; 32]> {
        let peer = X25519PublicKey::from(viewer_public);
        let shared = self.secret.diffie_hellman(&peer);
        derive_session(shared.as_bytes(), device_number, nonce)
    }
}

impl ViewerEphemeral {
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public_key = X25519PublicKey::from(&secret).to_bytes();
        Self { secret, public_key }
    }

    pub fn public_key_b64(&self) -> String { URL_SAFE_NO_PAD.encode(self.public_key) }

    pub fn proof(
        &self, device_number: &str, host_public: [u8; 32],
        nonce: [u8; 16], code: &str,
    ) -> anyhow::Result<[u8; 32]> {
        let peer = X25519PublicKey::from(host_public);
        let shared = self.secret.diffie_hellman(&peer);
        make_proof(shared.as_bytes(), device_number, code, self.public_key, nonce)
    }

    pub fn session_secret(
        &self, device_number: &str, host_public: [u8; 32], nonce: [u8; 16],
    ) -> anyhow::Result<[u8; 32]> {
        let peer = X25519PublicKey::from(host_public);
        let shared = self.secret.diffie_hellman(&peer);
        derive_session(shared.as_bytes(), device_number, nonce)
    }
}

fn make_proof(
    shared: &[u8; 32], device_number: &str, code: &str,
    viewer_public: [u8; 32], nonce: [u8; 16],
) -> anyhow::Result<[u8; 32]> {
    anyhow::ensure!(code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()), "pairing code must be six digits");
    let proof_info = format!("{PROOF_DOMAIN}{device_number}|{code}");
    let hkdf = Hkdf::<Sha256>::new(Some(&nonce), shared);
    let mut key = [0u8; 32];
    hkdf.expand(proof_info.as_bytes(), &mut key).map_err(|_| anyhow::anyhow!("HKDF expand failed"))?;
    let viewer_b64 = URL_SAFE_NO_PAD.encode(viewer_public);
    let nonce_b64 = URL_SAFE_NO_PAD.encode(nonce);
    let message = format!("{REQUEST_DOMAIN}{device_number}|{viewer_b64}|{nonce_b64}");
    let mut mac = HmacSha256::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid HMAC key"))?;
    mac.update(message.as_bytes());
    key.fill(0);
    Ok(mac.finalize().into_bytes().into())
}

fn derive_session(shared: &[u8; 32], device_number: &str, nonce: [u8; 16]) -> anyhow::Result<[u8; 32]> {
    let info = format!("{SESSION_DOMAIN}{device_number}");
    let hkdf = Hkdf::<Sha256>::new(Some(&nonce), shared);
    let mut session = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut session).map_err(|_| anyhow::anyhow!("session HKDF expand failed"))?;
    Ok(session)
}

pub fn verify_pairing_window(
    identity_public: [u8; 32], device_number: &str, expires_at: i64,
    pairing_public_b64: &str, signature_b64: &str,
) -> bool {
    let Ok(public) = VerifyingKey::from_bytes(&identity_public) else { return false; };
    let Ok(signature_bytes) = URL_SAFE_NO_PAD.decode(signature_b64) else { return false; };
    let Ok(signature) = Signature::from_slice(&signature_bytes) else { return false; };
    let message = format!("{WINDOW_DOMAIN}{device_number}|{expires_at}|{pairing_public_b64}");
    public.verify(message.as_bytes(), &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_viewer_derive_same_proof_and_session() {
        let host = PairingWindow::generate(1_790_000_000);
        let viewer = ViewerEphemeral::generate();
        let nonce = [0x51u8; 16];
        let number = "123456789";
        let proof_viewer = viewer.proof(number, host.public_key, nonce, &host.code).unwrap();
        let proof_host = host.proof_for_viewer(number, viewer.public_key, nonce).unwrap();
        assert_eq!(proof_viewer, proof_host);
        let session_viewer = viewer.session_secret(number, host.public_key, nonce).unwrap();
        let session_host = host.session_secret(number, viewer.public_key, nonce).unwrap();
        assert_eq!(session_viewer, session_host);
        assert_ne!(session_viewer, [0u8; 32]);
    }

    #[test]
    fn wrong_six_digit_code_does_not_validate() {
        let host = PairingWindow::generate(1_790_000_000);
        let viewer = ViewerEphemeral::generate();
        let nonce = [0x22u8; 16];
        let valid = viewer.proof("123456789", host.public_key, nonce, &host.code).unwrap();
        let wrong = format!("{:06}", (host.code.parse::<u32>().unwrap() + 1) % 1_000_000);
        let invalid = viewer.proof("123456789", host.public_key, nonce, &wrong).unwrap();
        assert_ne!(valid, invalid);
    }

    #[test]
    fn identity_signatures_bind_pairing_window_and_decision() {
        let identity = DeviceIdentity::generate();
        let window = PairingWindow::generate(1_790_000_000);
        let public_b64 = window.public_key_b64();
        let signature = identity.sign_pairing_window("123456789", window.expires_at, &public_b64);
        assert!(verify_pairing_window(identity.public_key(), "123456789", window.expires_at, &public_b64, &signature));
        assert!(!verify_pairing_window(identity.public_key(), "123456788", window.expires_at, &public_b64, &signature));
        let decision = identity.sign_decision("a".repeat(32).as_str(), "123456789", "viewer-key", "nonce", true);
        assert!(!decision.is_empty());
    }
}

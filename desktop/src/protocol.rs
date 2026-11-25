use aes_gcm::{aead::{Aead, KeyInit, Payload}, Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const FRAME: u8 = 1;
pub const CONTROL: u8 = 2;
pub const CLIPBOARD: u8 = 3;
pub const MONITORS: u8 = 4;
pub const AVC: u8 = 5;
// PING/PONG each carry only an opaque 8-byte nonce; GCM authenticates both.
pub const PING: u8 = 8;
pub const PONG: u8 = 9;

pub fn new_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    secret
}

/// Human-friendly numeric label for identifying a device.
// IMPORTANT: Not a password and never sufficient for connection authorization.
pub fn device_number(secret: &[u8;32]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"p2p-device-number-v1:");
    hash.update(secret);
    let digest = hash.finalize();
    let mut prefix = [0u8;8];
    prefix.copy_from_slice(&digest[..8]);
    format!("{:09}", u64::from_be_bytes(prefix) % 1_000_000_000)
}

pub fn room_id(secret: &[u8; 32]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"p2p-v02-room:");
    hash.update(secret);
    hex::encode(hash.finalize())
}

pub fn encrypt(secret: &[u8; 32], kind: u8, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    encrypt_with_nonce(secret, kind, plaintext, nonce)
}

fn encrypt_with_nonce(secret: &[u8;32], kind: u8, plaintext: &[u8], nonce: [u8;12]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(secret).map_err(|_| anyhow!("invalid key"))?;
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &[kind] })
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut result = Vec::with_capacity(1 + nonce.len() + ciphertext.len());
    result.push(kind);
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

pub fn decrypt(secret: &[u8; 32], packet: &[u8]) -> Result<(u8, Vec<u8>)> {
    if packet.len() < 30 || packet.len() > 3 * 1024 * 1024 { return Err(anyhow!("invalid packet length")); }
    let cipher = Aes256Gcm::new_from_slice(secret).map_err(|_| anyhow!("invalid key"))?;
    let plaintext = cipher.decrypt(Nonce::from_slice(&packet[1..13]), Payload { msg: &packet[13..], aad: &packet[..1] })
        .map_err(|_| anyhow!("unauthenticated packet"))?;
    Ok((packet[0], plaintext))
}

// LAN v4 uses direction-separated GCM nonces and monotonic counters.
// Never reset these counters without deriving a fresh connection key.
pub(crate) const HOST_DIRECTION: [u8;4] = *b"HOST";
pub(crate) const VIEW_DIRECTION: [u8;4] = *b"VIEW";
const MAX_SEQUENCE: u64 = 0x7fff_ffff;

pub(crate) struct SequenceSender {
    direction: [u8;4],
    last: u64,
}
impl SequenceSender {
    pub(crate) fn new(direction: [u8;4]) -> Self { Self { direction, last: 0 } }
    pub(crate) fn encrypt(&mut self, secret: &[u8;32], kind: u8, bytes: &[u8]) -> Result<Vec<u8>> {
        anyhow::ensure!(self.last < MAX_SEQUENCE, "LAN sequence exhausted");
        self.last += 1;
        let mut nonce = [0u8;12];
        nonce[..4].copy_from_slice(&self.direction);
        nonce[4..].copy_from_slice(&self.last.to_be_bytes());
        encrypt_with_nonce(secret, kind, bytes, nonce)
    }
}

/// Call only after GCM authentication. WebSocket preserves packet ordering;
/// gaps are allowed because a decoder may intentionally skip video packets.
pub(crate) struct ReplayGuard {
    direction: [u8;4],
    last: u64,
}
impl ReplayGuard {
    pub(crate) fn new(direction: [u8;4]) -> Self { Self { direction, last: 0 } }
    pub(crate) fn accept(&mut self, packet: &[u8]) -> bool {
        let Some(nonce) = packet.get(1..13) else { return false; };
        if nonce[..4] != self.direction { return false; }
        let sequence = u64::from_be_bytes(nonce[4..].try_into().unwrap());
        if sequence <= self.last || sequence > MAX_SEQUENCE { return false; }
        self.last = sequence;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_roundtrip_and_reject_modified() {
        let secret = new_secret();
        let mut packet = encrypt(&secret, CONTROL, br#"{"type":"move"}"#).unwrap();
        assert_eq!(decrypt(&secret, &packet).unwrap().0, CONTROL);
        let mut wrong_type = packet.clone();
        wrong_type[0] = FRAME;
        assert!(decrypt(&secret, &wrong_type).is_err(), "packet type is authenticated");
        *packet.last_mut().unwrap() ^= 1;
        assert!(decrypt(&secret, &packet).is_err());
    }
    #[test]
    fn ordered_nonce_guard_rejects_replays_and_wrong_direction() {
        let key = new_secret();
        let mut tx = SequenceSender::new(VIEW_DIRECTION);
        let mut guard = ReplayGuard::new(VIEW_DIRECTION);
        let a = tx.encrypt(&key, CONTROL, b"first").unwrap();
        let b = tx.encrypt(&key, CONTROL, b"second").unwrap();
        let c = tx.encrypt(&key, CONTROL, b"third").unwrap();
        let reflected = SequenceSender::new(HOST_DIRECTION).encrypt(&key, CONTROL, b"other").unwrap();
        assert!(!guard.accept(&[]));
        assert!(decrypt(&key, &a).is_ok());
        assert!(guard.accept(&a));
        assert!(!guard.accept(&a));
        assert!(!guard.accept(&reflected));
        assert!(guard.accept(&c));
        assert!(!guard.accept(&b));
        assert!(!guard.accept(&c));
        let mut near = SequenceSender { direction: VIEW_DIRECTION, last: MAX_SEQUENCE-1 };
        let last = near.encrypt(&key, CONTROL, b"last").unwrap();
        assert!(guard.accept(&last));
        assert!(near.encrypt(&key, CONTROL, b"overflow").is_err());
    }

    #[test]
    fn numeric_device_label_is_stable_and_not_credential() {
        let a = [0x52u8;32];
        let b = [0x53u8;32];
        let number = device_number(&a);
        assert_eq!(number.len(),9);
        assert!(number.bytes().all(|c| c.is_ascii_digit()));
        assert_eq!(number, device_number(&a));
        assert_ne!(number, device_number(&b));
    }
    #[test]
    fn room_is_stable_but_not_raw_key() {
        let key = [7u8; 32];
        assert_eq!(room_id(&key).len(), 64);
        assert_ne!(room_id(&key), hex::encode(key));
    }
}

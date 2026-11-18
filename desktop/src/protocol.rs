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
    let cipher = Aes256Gcm::new_from_slice(secret).map_err(|_| anyhow!("invalid key"))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
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

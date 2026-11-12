//! Experimental V0.10 UDP JPEG-preview packet framing. Every fragment is
//! encrypted and authenticated individually with the existing AES-GCM key.
//! Media never includes mouse, clipboard, session keys, or relay credentials.
use crate::protocol;
use anyhow::{anyhow, Result};

pub const VIDEO: u8 = 14;
pub const MAX_FRAME_BYTES: usize = 96 * 1024;
pub const CHUNK_BYTES: usize = 900;
pub const MAX_CHUNKS: usize = 110;
const HEADER: usize = 12; // frame_id:u32, count:u16, index:u16, length:u32

#[derive(Debug, PartialEq, Eq)]
pub struct Fragment<'a> {
    pub frame_id: u32,
    pub count: u16,
    pub index: u16,
    pub total: u32,
    pub bytes: &'a [u8],
}

pub fn fragment_header(buffer: &[u8]) -> Option<Fragment<'_>> {
    if buffer.len() <= HEADER || buffer.len() > HEADER + CHUNK_BYTES {
        return None;
    }
    let frame_id = u32::from_be_bytes(buffer[..4].try_into().ok()?);
    let count = u16::from_be_bytes(buffer[4..6].try_into().ok()?);
    let index = u16::from_be_bytes(buffer[6..8].try_into().ok()?);
    let total = u32::from_be_bytes(buffer[8..12].try_into().ok()?);
    if frame_id == 0 || count == 0 || count as usize > MAX_CHUNKS
        || index >= count || total == 0 || total as usize > MAX_FRAME_BYTES
        || count as usize != (total as usize).div_ceil(CHUNK_BYTES)
    {
        return None;
    }
    let start = index as usize * CHUNK_BYTES;
    let expected = (total as usize - start).min(CHUNK_BYTES);
    if buffer.len() - HEADER != expected { return None; }
    Some(Fragment {
        frame_id, count, index, total, bytes: &buffer[HEADER..],
    })
}

pub fn encode_jpeg(secret: &[u8;32], frame_id: u32, jpeg: &[u8]) -> Result<Vec<Vec<u8>>> {
    if frame_id == 0 || jpeg.len() < 4 || jpeg.len() > MAX_FRAME_BYTES
        || !jpeg.starts_with(&[0xff, 0xd8]) || !jpeg.ends_with(&[0xff, 0xd9])
    {
        return Err(anyhow!("invalid or oversized UDP JPEG frame"));
    }
    let total = jpeg.len();
    let count = total.div_ceil(CHUNK_BYTES);
    if count > MAX_CHUNKS { return Err(anyhow!("too many UDP fragments")); }
    let mut output = Vec::with_capacity(count);
    for (index, chunk) in jpeg.chunks(CHUNK_BYTES).enumerate() {
        let mut body = Vec::with_capacity(HEADER + chunk.len());
        body.extend_from_slice(&frame_id.to_be_bytes());
        body.extend_from_slice(&(count as u16).to_be_bytes());
        body.extend_from_slice(&(index as u16).to_be_bytes());
        body.extend_from_slice(&(total as u32).to_be_bytes());
        body.extend_from_slice(chunk);
        output.push(protocol::encrypt(secret, VIDEO, &body)?);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconstructs_and_authenticates_all_fragments() {
        let key = protocol::new_secret();
        let mut jpeg = vec![1u8; 3050];
        jpeg[..2].copy_from_slice(&[0xff, 0xd8]);
        jpeg[3048..].copy_from_slice(&[0xff, 0xd9]);
        let packets = encode_jpeg(&key, 16, &jpeg).unwrap();
        assert_eq!(packets.len(), 4);
        let mut reconstructed = Vec::new();
        for (i, pkt) in packets.iter().enumerate() {
            assert!(pkt.len() <= 1000, "bounded UDP MTU");
            let (kind, plaintext) = protocol::decrypt(&key, pkt).unwrap();
            assert_eq!(kind, VIDEO);
            let f = fragment_header(&plaintext).unwrap();
            assert_eq!((f.frame_id, f.index), (16, i as u16));
            reconstructed.extend_from_slice(f.bytes);
            let mut damaged = pkt.clone();
            *damaged.last_mut().unwrap() ^= 1;
            assert!(protocol::decrypt(&key, &damaged).is_err());
        }
        assert_eq!(reconstructed, jpeg);
    }
    #[test]
    fn refuses_invalid_geometry_or_payload() {
        let key = protocol::new_secret();
        assert!(encode_jpeg(&key, 0, &[0xff,0xd8,0xff,0xd9]).is_err());
        assert!(encode_jpeg(&key, 1, &vec![0xff; 100*1024]).is_err());
        assert!(fragment_header(&[0u8; 9]).is_none());
        let mut header = vec![0u8; 13];
        header[..4].copy_from_slice(&1u32.to_be_bytes());
        header[4..6].copy_from_slice(&1u16.to_be_bytes());
        header[8..12].copy_from_slice(&900u32.to_be_bytes());
        assert!(fragment_header(&header).is_none());
    }
}

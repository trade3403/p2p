//! Optional LAN-only P2P WebSocket transport.
//!
//! This is **NOT** STUN/ICE NAT traversal. Both devices must share a reachable
//! private IPv4 network, and the owner's firewall must allow TCP 43821.
//! No bearer tokens or session keys ever appear in a URL or HTTP header.
//! An encrypted proof-of-key challenge is required before sharing frames.
use crate::{host::{self, HostConfig}, protocol};
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::{
    net::IpAddr,
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

pub const PORT: u16 = 43821;
pub const HELLO: u8 = 6;
pub const READY: u8 = 7;
const HELLO_PAYLOAD: &[u8] = b"P2P-LAN-HELLO-v3";
const CHALLENGE_PAYLOAD: &[u8] = b"P2P-LAN-CHALLENGE-v3";
const PROOF_PAYLOAD: &[u8] = b"P2P-LAN-PROOF-v3";
const READY_PAYLOAD: &[u8] = b"P2P-LAN-READY-v3";
const NONCE_LEN: usize = 32;

fn transcript(prefix: &[u8], client: &[u8], server: &[u8]) -> Vec<u8> {
    [prefix, client, server].concat()
}

// Domain separation binds the business key to both fresh nonces.
fn business_key(secret: &[u8;32], client: &[u8], server: &[u8]) -> [u8;32] {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).expect("32-byte HMAC key");
    mac.update(b"P2P-LAN-BUSINESS-v3");
    mac.update(client);
    mac.update(server);
    mac.finalize().into_bytes().into()
}

fn hello_nonce(secret: &[u8;32], packet: &[u8]) -> Option<[u8; NONCE_LEN]> {
    let (kind, bytes) = protocol::decrypt(secret, packet).ok()?;
    if kind != HELLO { return None; }
    bytes.strip_prefix(HELLO_PAYLOAD)?.try_into().ok()
}

fn matches_proof(secret: &[u8;32], packet: &[u8], client: &[u8], server: &[u8]) -> bool {
    matches!(protocol::decrypt(secret, packet),
        Ok((HELLO, bytes)) if bytes == transcript(PROOF_PAYLOAD, client, server))
}

pub fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => addr.is_private() || addr.is_loopback(),
        IpAddr::V6(_) => false,
    }
}

pub fn authorized_hello(secret: &[u8;32], packet: &[u8]) -> bool {
    hello_nonce(secret, packet).is_some()
}

async fn accept_authorized(
    stream: TcpStream,
    config: &HostConfig,
) -> Result<(WebSocketStream<TcpStream>, [u8;32])> {
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return Err(anyhow!("LAN sharing is not active"));
    }
    let mut socket = tokio::time::timeout(Duration::from_secs(4), accept_async(stream))
        .await.context("LAN websocket handshake timed out")??;
    let incoming = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await.context("LAN key proof timed out")?
        .ok_or_else(|| anyhow!("LAN client disconnected before authorization"))??;
    let Message::Binary(packet) = incoming else {
        return Err(anyhow!("LAN authorization requires encrypted binary hello"));
    };
    let client_nonce = hello_nonce(&config.secret, &packet)
        .ok_or_else(|| anyhow!("invalid LAN v3 hello"))?;
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return Err(anyhow!("LAN sharing was stopped during authorization"));
    }
    // Fresh random host challenge for each socket; captured proofs cannot
    // authorize a different socket, even when the initial hello is replayed.
    let server_nonce = protocol::new_secret();
    let challenge = protocol::encrypt(&config.secret, READY,
        &transcript(CHALLENGE_PAYLOAD, &client_nonce, &server_nonce))?;
    tokio::time::timeout(Duration::from_secs(4), socket.send(Message::Binary(challenge)))
        .await.context("LAN challenge send timed out")??;
    let incoming = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await.context("LAN challenge proof timed out")?
        .ok_or_else(|| anyhow!("LAN client disconnected before proof"))??;
    let Message::Binary(proof) = incoming else {
        return Err(anyhow!("LAN challenge requires encrypted binary proof"));
    };
    if !matches_proof(&config.secret, &proof, &client_nonce, &server_nonce) {
        return Err(anyhow!("invalid LAN challenge proof"));
    }
    // A stop/disable may occur while the handshake is awaiting network I/O.
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return Err(anyhow!("LAN sharing was stopped during authorization"));
    }
    let ack = protocol::encrypt(&config.secret, READY,
        &transcript(READY_PAYLOAD, &client_nonce, &server_nonce))?;
    tokio::time::timeout(Duration::from_secs(4), socket.send(Message::Binary(ack)))
        .await.context("LAN ready send timed out")??;
    Ok((socket, business_key(&config.secret, &client_nonce, &server_nonce)))
}

pub fn spawn(config: HostConfig) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(_) => return,
        };
        let run = async {
            let listener = TcpListener::bind(("0.0.0.0", PORT)).await
                .context("bind LAN direct port 43821")?;
            let _ = config.messages.send(format!(
                "局域网直连已启用 · TCP {} · 仅接收私有 IPv4 和加密配对",
                PORT
            ));
            while config.running.load(Ordering::SeqCst) && config.lan_direct.load(Ordering::SeqCst) {
                tokio::select! {
                    incoming = listener.accept() => {
                        let (stream, address) = incoming?;
                        if !is_lan_address(address.ip()) {
                            continue;
                        }
                        match accept_authorized(stream, &config).await {
                            Ok((socket, key)) => {
                                if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
                                    continue;
                                }
                                let _ = config.messages.send(format!(
                                    "局域网直连已验证 · 对端 {}",
                                    address.ip()
                                ));
                                // One authorized LAN viewer per host at a time.
                                // Relay remains connected independently for fallback.
                                config.lan_active.store(true, Ordering::SeqCst);
                                let mut lan_config = config.clone();
                                lan_config.secret = key;
                                let result = host::serve_session(&lan_config, socket, true).await;
                                config.lan_active.store(false, Ordering::SeqCst);
                                if let Err(error) = result {
                                    let _ = config.messages.send(format!(
                                        "局域网直连断开：{}",
                                        error
                                    ));
                                }
                            }
                            Err(_) => {
                                // Do not echo untrusted handshakes or secret-related
                                // error details to the UI or logs.
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                }
            }
            Ok::<(), anyhow::Error>(())
        };
        if let Err(error) = runtime.block_on(run) {
            let _ = config.messages.send(format!(
                "局域网直连不可用（VPS 中转不受影响）：{}",
                error
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    #[test]
    fn accepts_only_private_ipv4_peers() {
        for ip in [Ipv4Addr::new(192,168,1,5),Ipv4Addr::new(10,0,0,2),Ipv4Addr::new(172,16,3,4),Ipv4Addr::LOCALHOST] {
            assert!(is_lan_address(IpAddr::V4(ip)));
        }
        for ip in [Ipv4Addr::new(8,8,8,8),Ipv4Addr::new(172,32,1,1),Ipv4Addr::new(1,1,1,1)] {
            assert!(!is_lan_address(IpAddr::V4(ip)));
        }
        assert!(!is_lan_address(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }
    #[tokio::test]
    async fn loopback_websocket_requires_encrypted_proof_before_ready() {
        exercise_handshake(false).await;
    }

    #[tokio::test]
    async fn loopback_websocket_rejects_proof_after_sharing_stops() {
        exercise_handshake(true).await;
    }

    async fn exercise_handshake(stop_before_proof: bool) {
        use std::sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize}, Arc, mpsc};
        use tokio_tungstenite::connect_async;

        let key = protocol::new_secret();
        let (messages, _rx) = mpsc::channel();
        let config = HostConfig {
            server: "wss://example.com".into(), relay_token: String::new(), secret: key,
            running: Arc::new(AtomicBool::new(true)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(true)),
            lan_active: Arc::new(AtomicBool::new(false)),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            messages,
        };
        let running = config.running.clone();
        let listener = TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok((mut socket, key)) = accept_authorized(stream, &config).await else { return false; };
            // Exercise real encrypted WebSocket traffic after READY, without a Relay.
            // This is a transport integration test, not desktop input injection.
            let request = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await.unwrap().unwrap().unwrap().into_data();
            let (kind, payload) = protocol::decrypt(&key, &request).unwrap();
            assert_eq!(kind, protocol::PING);
            assert_eq!(payload, 42u64.to_be_bytes());
            socket.send(Message::Binary(protocol::encrypt(&key, protocol::PONG, &payload).unwrap()))
                .await.unwrap();
            for expected in [br#"{"type":"move","x":0.5,"y":0.5}"#.as_slice(),
                br#"{"type":"clipboard_set","text":"LAN test"}"#.as_slice()] {
                let packet = tokio::time::timeout(Duration::from_secs(5), socket.next())
                    .await.unwrap().unwrap().unwrap().into_data();
                let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
                assert_eq!(kind, protocol::CONTROL);
                assert_eq!(payload.as_slice(), expected);
            }
            // Encode actual JPEG bytes rather than a fabricated video payload.
            let pixels = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                .encode(&pixels, 2, 2, image::ColorType::Rgb8).unwrap();
            socket.send(Message::Binary(protocol::encrypt(&key, protocol::FRAME, &jpeg).unwrap()))
                .await.unwrap();
            socket.send(Message::Binary(protocol::encrypt(&key, protocol::CLIPBOARD, b"LAN test").unwrap()))
                .await.unwrap();
            true
        });
        let (mut viewer, _) = connect_async(format!("ws://{address}/direct")).await.unwrap();
        let client = protocol::new_secret();
        viewer.send(Message::Binary(protocol::encrypt(&key, HELLO,
            &transcript(HELLO_PAYLOAD, &client, &[])).unwrap())).await.unwrap();
        let challenge = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, bytes) = protocol::decrypt(&key, &challenge).unwrap();
        assert_eq!(kind, READY);
        let prefix = transcript(CHALLENGE_PAYLOAD, &client, &[]);
        let server = bytes.strip_prefix(prefix.as_slice()).unwrap();
        assert_eq!(server.len(), NONCE_LEN);
        if stop_before_proof {
            running.store(false, Ordering::SeqCst);
        }
        viewer.send(Message::Binary(protocol::encrypt(&key, HELLO,
            &transcript(PROOF_PAYLOAD, &client, server)).unwrap())).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), viewer.next()).await.unwrap();
        if stop_before_proof {
            assert!(!matches!(reply, Some(Ok(Message::Binary(_)))));
            assert!(!task.await.unwrap());
            return;
        }
        let ack = reply.unwrap().unwrap().into_data();
        let (kind, contents) = protocol::decrypt(&key, &ack).unwrap();
        assert_eq!(kind, READY);
        assert_eq!(contents, transcript(READY_PAYLOAD, &client, server));
        let key = business_key(&key, &client, server);
        viewer.send(Message::Binary(protocol::encrypt(&key, protocol::PING,
            &42u64.to_be_bytes()).unwrap())).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &reply).unwrap();
        assert_eq!(kind, protocol::PONG);
        assert_eq!(payload, 42u64.to_be_bytes());
        for command in [br#"{"type":"move","x":0.5,"y":0.5}"#.as_slice(),
            br#"{"type":"clipboard_set","text":"LAN test"}"#.as_slice()] {
            viewer.send(Message::Binary(protocol::encrypt(&key, protocol::CONTROL, command).unwrap()))
                .await.unwrap();
        }
        let packet = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
        assert_eq!(kind, protocol::FRAME);
        let decoded = image::load_from_memory_with_format(&payload, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
        let packet = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
        assert_eq!(kind, protocol::CLIPBOARD);
        assert_eq!(payload.as_slice(), b"LAN test");
        assert!(task.await.unwrap());
    }

    #[test]
    fn encrypted_hello_is_required() {
        let key = protocol::new_secret();
        let valid = protocol::encrypt(&key, HELLO, &transcript(HELLO_PAYLOAD, &[1;32], &[])).unwrap();
        let legacy = protocol::encrypt(&key, HELLO, b"P2P-LAN-HELLO-v1").unwrap();
        assert!(!authorized_hello(&key, &legacy));
        assert!(authorized_hello(&key, &valid));
        assert!(!authorized_hello(&protocol::new_secret(), &valid));
        let wrong = protocol::encrypt(&key, protocol::CONTROL, HELLO_PAYLOAD).unwrap();
        assert!(!authorized_hello(&key, &wrong));
        let wrong_message = protocol::encrypt(&key, HELLO, b"hello").unwrap();
        assert!(!authorized_hello(&key, &wrong_message));
        let mut tampered = valid;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(!authorized_hello(&key, &tampered));
    }

    #[test]
    fn old_business_ciphertext_rejected_under_new_connection_key() {
        let root = [7;32];
        let first = business_key(&root, &[1;32], &[2;32]);
        let second = business_key(&root, &[1;32], &[3;32]);
        assert_ne!(first, second);
        assert_ne!(first, root);
        let packet = protocol::encrypt(&first, protocol::CONTROL, b"command").unwrap();
        assert!(protocol::decrypt(&first, &packet).is_ok());
        assert!(protocol::decrypt(&second, &packet).is_err());
        assert!(protocol::decrypt(&root, &packet).is_err());
    }

    #[test]
    fn captured_proof_cannot_authorize_a_new_transcript() {
        let key = protocol::new_secret();
        let client = [1;32];
        let old_server = [2;32];
        let new_server = [3;32];
        let proof = protocol::encrypt(&key, HELLO,
            &transcript(PROOF_PAYLOAD, &client, &old_server)).unwrap();
        assert!(matches_proof(&key, &proof, &client, &old_server));
        assert!(!matches_proof(&key, &proof, &client, &new_server));
        assert!(!matches_proof(&key, &proof, &[4;32], &old_server));
        let reflected = protocol::encrypt(&key, READY,
            &transcript(CHALLENGE_PAYLOAD, &client, &old_server)).unwrap();
        assert!(!matches_proof(&key, &reflected, &client, &old_server));
    }
}

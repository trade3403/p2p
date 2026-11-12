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
const HELLO_PAYLOAD: &[u8] = b"P2P-LAN-HELLO-v1";
const READY_PAYLOAD: &[u8] = b"P2P-LAN-READY-v1";

pub fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => addr.is_private() || addr.is_loopback(),
        IpAddr::V6(_) => false,
    }
}

pub fn authorized_hello(secret: &[u8;32], packet: &[u8]) -> bool {
    match protocol::decrypt(secret, packet) {
        Ok((kind, bytes)) => kind == HELLO && bytes.as_slice() == HELLO_PAYLOAD,
        Err(_) => false,
    }
}

async fn accept_authorized(
    stream: TcpStream,
    config: &HostConfig,
) -> Result<WebSocketStream<TcpStream>> {
    let mut socket = tokio::time::timeout(Duration::from_secs(4), accept_async(stream))
        .await.context("LAN websocket handshake timed out")??;
    let incoming = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await.context("LAN key proof timed out")?
        .ok_or_else(|| anyhow!("LAN client disconnected before authorization"))??;
    let Message::Binary(packet) = incoming else {
        return Err(anyhow!("LAN authorization requires encrypted binary hello"));
    };
    if !authorized_hello(&config.secret, &packet) {
        return Err(anyhow!("LAN client did not prove session key"));
    }
    let ack = protocol::encrypt(&config.secret, READY, READY_PAYLOAD)?;
    socket.send(Message::Binary(ack)).await?;
    Ok(socket)
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
            while config.running.load(Ordering::SeqCst) {
                tokio::select! {
                    incoming = listener.accept() => {
                        let (stream, address) = incoming?;
                        if !is_lan_address(address.ip()) {
                            continue;
                        }
                        match accept_authorized(stream, &config).await {
                            Ok(socket) => {
                                let _ = config.messages.send(format!(
                                    "局域网直连已验证 · 对端 {}",
                                    address.ip()
                                ));
                                // One authorized LAN viewer per host at a time.
                                // Relay remains connected independently for fallback.
                                config.lan_active.store(true, Ordering::SeqCst);
                                let result = host::serve_session(&config, socket, true).await;
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
        let listener = TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            accept_authorized(stream, &config).await.is_ok()
        });
        let (mut viewer, _) = connect_async(format!("ws://{address}/direct")).await.unwrap();
        viewer.send(Message::Binary(protocol::encrypt(&key, HELLO, HELLO_PAYLOAD).unwrap())).await.unwrap();
        let ack = viewer.next().await.unwrap().unwrap().into_data();
        let (kind, contents) = protocol::decrypt(&key, &ack).unwrap();
        assert_eq!(kind, READY);
        assert_eq!(contents, READY_PAYLOAD);
        assert!(task.await.unwrap());
    }

    #[test]
    fn encrypted_hello_is_required() {
        let key = protocol::new_secret();
        let valid = protocol::encrypt(&key, HELLO, HELLO_PAYLOAD).unwrap();
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
}

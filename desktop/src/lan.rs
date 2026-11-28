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
const HELLO_PAYLOAD: &[u8] = b"P2P-LAN-HELLO-v4";
const CHALLENGE_PAYLOAD: &[u8] = b"P2P-LAN-CHALLENGE-v4";
const PROOF_PAYLOAD: &[u8] = b"P2P-LAN-PROOF-v4";
const READY_PAYLOAD: &[u8] = b"P2P-LAN-READY-v4";
const NONCE_LEN: usize = 32;
const HELLO_V5: &[u8] = b"P2P-LAN-HELLO-v5";
const CHALLENGE_V5: &[u8] = b"P2P-LAN-CHALLENGE-v5";
const PROOF_V5: &[u8] = b"P2P-LAN-PROOF-v5";
const PREPARED_V5: &[u8] = b"P2P-LAN-PREPARED-v5";
const COMMIT_V5: &[u8] = b"P2P-LAN-COMMIT-v5";
const READY_V5: &[u8] = b"P2P-LAN-READY-v5";

#[derive(Clone, Copy, PartialEq, Eq)]
enum HandshakeVersion { V4, V5 }

impl HandshakeVersion {
    fn hello(self) -> &'static [u8] { match self { Self::V4 => HELLO_PAYLOAD, Self::V5 => HELLO_V5 } }
    fn challenge(self) -> &'static [u8] { match self { Self::V4 => CHALLENGE_PAYLOAD, Self::V5 => CHALLENGE_V5 } }
    fn proof(self) -> &'static [u8] { match self { Self::V4 => PROOF_PAYLOAD, Self::V5 => PROOF_V5 } }
    fn ready(self) -> &'static [u8] { match self { Self::V4 => READY_PAYLOAD, Self::V5 => READY_V5 } }
}

fn transcript(prefix: &[u8], client: &[u8], server: &[u8]) -> Vec<u8> {
    [prefix, client, server].concat()
}

// Domain separation binds the business key to both fresh nonces.
fn business_key(secret: &[u8;32], client: &[u8], server: &[u8]) -> [u8;32] {
    versioned_business_key(secret, client, server, HandshakeVersion::V4)
}

fn versioned_business_key(secret: &[u8;32], client: &[u8], server: &[u8], version: HandshakeVersion) -> [u8;32] {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).expect("32-byte HMAC key");
    mac.update(match version {
        HandshakeVersion::V4 => b"P2P-LAN-BUSINESS-v4",
        HandshakeVersion::V5 => b"P2P-LAN-BUSINESS-v5",
    });
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

const MAX_ADVERTISED_IPV4: usize = 16;

fn advertised_ipv4(primary: Option<std::net::Ipv4Addr>, mut candidates: Vec<std::net::Ipv4Addr>) -> Vec<String> {
    let usable = |ip: &std::net::Ipv4Addr| ip.is_private() && !matches!(ip.octets()[3], 0 | 255);
    candidates.retain(usable);
    candidates.sort_unstable();
    candidates.dedup();
    if let Some(primary) = primary.filter(usable) {
        candidates.retain(|ip| *ip != primary);
        candidates.insert(0, primary);
    }
    candidates.into_iter().take(MAX_ADVERTISED_IPV4).map(|ip| ip.to_string()).collect()
}

// The legacy default-route hint remains first for older viewers. All other
// candidates come from up interfaces; no remote probing or DNS is needed.
#[cfg(windows)]
pub(crate) fn local_ipv4_candidates() -> Vec<String> {
    extern "C" { fn p2p_lan_ipv4_candidates(output: *mut u32, capacity: u32) -> u32; }
    let mut raw = [0u32; 64];
    let count = unsafe { p2p_lan_ipv4_candidates(raw.as_mut_ptr(), raw.len() as u32) } as usize;
    let candidates = raw[..count.min(raw.len())].iter().copied().map(std::net::Ipv4Addr::from).collect();
    let primary = std::net::UdpSocket::bind("0.0.0.0:0").ok().and_then(|socket| {
        socket.connect("1.1.1.1:443").ok()?;
        match socket.local_addr().ok()?.ip() {
            IpAddr::V4(ip) => Some(ip),
            _ => None,
        }
    });
    advertised_ipv4(primary, candidates)
}

pub fn authorized_hello(secret: &[u8;32], packet: &[u8]) -> bool {
    hello_nonce(secret, packet).is_some()
}

// Decode only the two supported literal HELLO transcripts. Version is bound
// to every subsequent message and the derived business key.
fn versioned_hello(secret: &[u8;32], packet: &[u8]) -> Option<(HandshakeVersion, [u8;32])> {
    let (kind, bytes) = protocol::decrypt(secret, packet).ok()?;
    if kind != HELLO { return None; }
    for version in [HandshakeVersion::V4, HandshakeVersion::V5] {
        if let Some(nonce) = bytes.strip_prefix(version.hello()) {
            return Some((version, nonce.try_into().ok()?));
        }
    }
    None
}

fn matches_step(secret: &[u8;32], packet: &[u8], prefix: &[u8], client: &[u8], server: &[u8]) -> bool {
    matches!(protocol::decrypt(secret, packet),
        Ok((HELLO, bytes)) if bytes == transcript(prefix, client, server))
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
    let (version, client_nonce) = versioned_hello(&config.secret, &packet)
        .ok_or_else(|| anyhow!("invalid LAN hello"))?;
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return Err(anyhow!("LAN sharing was stopped during authorization"));
    }
    // Fresh random host challenge for each socket; captured proofs cannot
    // authorize a different socket, even when the initial hello is replayed.
    let server_nonce = protocol::new_secret();
    let challenge = protocol::encrypt(&config.secret, READY,
        &transcript(version.challenge(), &client_nonce, &server_nonce))?;
    tokio::time::timeout(Duration::from_secs(4), socket.send(Message::Binary(challenge)))
        .await.context("LAN challenge send timed out")??;
    let incoming = tokio::time::timeout(Duration::from_secs(4), socket.next())
        .await.context("LAN challenge proof timed out")?
        .ok_or_else(|| anyhow!("LAN client disconnected before proof"))??;
    let Message::Binary(proof) = incoming else {
        return Err(anyhow!("LAN challenge requires encrypted binary proof"));
    };
    if !matches_step(&config.secret, &proof, version.proof(), &client_nonce, &server_nonce) {
        return Err(anyhow!("invalid LAN challenge proof"));
    }
    // A stop/disable may occur while the handshake is awaiting network I/O.
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return Err(anyhow!("LAN sharing was stopped during authorization"));
    }
    if version == HandshakeVersion::V5 {
        // PREPARED authenticates the candidate but never grants video or
        // input ownership. The relay remains active while the peer decides.
        let prepared = protocol::encrypt(&config.secret, READY,
            &transcript(PREPARED_V5, &client_nonce, &server_nonce))?;
        tokio::time::timeout(Duration::from_secs(4), socket.send(Message::Binary(prepared)))
            .await.context("LAN prepared send timed out")??;
        let incoming = tokio::time::timeout(Duration::from_secs(4), socket.next())
            .await.context("LAN commit timed out")?
            .ok_or_else(|| anyhow!("LAN candidate closed before commit"))??;
        let Message::Binary(commit) = incoming else {
            return Err(anyhow!("LAN commit requires encrypted binary message"));
        };
        if !matches_step(&config.secret, &commit, COMMIT_V5, &client_nonce, &server_nonce) {
            return Err(anyhow!("invalid LAN commit"));
        }
        if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
            return Err(anyhow!("LAN sharing stopped before commit"));
        }
        if config.lan_active.load(Ordering::SeqCst) {
            return Err(anyhow!("LAN ownership already active"));
        }
    }
    let ack = protocol::encrypt(&config.secret, READY,
        &transcript(version.ready(), &client_nonce, &server_nonce))?;
    tokio::time::timeout(Duration::from_secs(4), socket.send(Message::Binary(ack)))
        .await.context("LAN ready send timed out")??;
    Ok((socket, versioned_business_key(&config.secret, &client_nonce, &server_nonce, version)))
}

// Ownership is scoped to the authorized session future, including error,
// cancellation and unwinding paths. A failed acquisition cannot clear a
// different session's flag.
struct ActiveLan<'a> {
    active: &'a std::sync::atomic::AtomicBool,
    gate: &'a host::ControlGate,
}
impl<'a> ActiveLan<'a> {
    fn acquire(active: &'a std::sync::atomic::AtomicBool, gate: &'a host::ControlGate) -> Option<Self> {
        let mut state = gate.lock();
        if active.load(Ordering::SeqCst) { return None; }
        state.advance();
        active.store(true, Ordering::SeqCst);
        gate.advance_video_epoch();
        Some(Self { active, gate })
    }
}
impl Drop for ActiveLan<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.advance();
        self.active.store(false, Ordering::SeqCst);
        self.gate.advance_video_epoch();
    }
}

// Shared by the real listener and the Windows loopback integration test.
// The active flag can only be reached with a socket returned by accept_authorized.
async fn serve_authorized_session(
    config: &HostConfig,
    socket: WebSocketStream<TcpStream>,
    key: [u8;32],
    address: IpAddr,
) {
    if !config.running.load(Ordering::SeqCst) || !config.lan_direct.load(Ordering::SeqCst) {
        return;
    }
    // The production listener serializes handshakes, but also enforce exclusive
    // ownership here so a competing caller cannot overwrite or clear it.
    let Some(_active) = ActiveLan::acquire(&config.lan_active, &config.control_gate) else { return; };
    let _ = config.messages.send(format!("局域网直连已验证 · 对端 {}", address));
    let mut lan_config = config.clone();
    lan_config.secret = key;
    let result = host::serve_session(&lan_config, socket, true).await;
    if let Err(error) = result {
        let _ = config.messages.send(format!("局域网直连断开：{}", error));
    }
}

#[cfg(all(test, windows))]
#[path = "lan_g1_test.rs"]
mod g1;

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
                                serve_authorized_session(&config, socket, key, address.ip()).await;
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
    fn active_lan_lease_is_exclusive_and_releases_on_drop() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let gate = host::ControlGate::default();
        let owner = ActiveLan::acquire(&flag, &gate).unwrap();
        assert!(flag.load(Ordering::SeqCst));
        assert!(ActiveLan::acquire(&flag, &gate).is_none());
        assert!(flag.load(Ordering::SeqCst));
        drop(owner);
        assert!(!flag.load(Ordering::SeqCst));
        let next = ActiveLan::acquire(&flag, &gate).unwrap();
        drop(next);
        assert!(!flag.load(Ordering::SeqCst));
    }
    #[test]
    fn transport_video_epoch_tracks_only_successful_lan_lease_transitions() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let input = std::sync::atomic::AtomicBool::new(false);
        let gate = host::ControlGate::default();
        let epoch = gate.video_epoch();
        let original = epoch.load(Ordering::SeqCst);
        {
            let _owner = ActiveLan::acquire(&flag, &gate).unwrap();
            assert_eq!(epoch.load(Ordering::SeqCst), original + 1);
            assert!(ActiveLan::acquire(&flag, &gate).is_none());
            gate.set_input(&input, true);
            gate.set_input(&input, false);
            gate.set_clipboard(&input, true);
            assert_eq!(epoch.load(Ordering::SeqCst), original + 1);
        }
        assert!(!flag.load(Ordering::SeqCst));
        assert_eq!(epoch.load(Ordering::SeqCst), original + 2);
        // A previously captured frame remains stale even after the flag returns
        // to the same false value, with no sampling during the lease.
        assert_ne!(original, epoch.load(Ordering::SeqCst));
        drop(ActiveLan::acquire(&flag, &gate).unwrap());
        assert_eq!(epoch.load(Ordering::SeqCst), original + 4);
    }

    #[test]
    fn advertised_candidates_are_private_deduplicated_bounded_and_primary_first() {
        let primary = Ipv4Addr::new(192,168,9,2);
        let candidates = vec![Ipv4Addr::new(10,0,0,2), primary, primary,
            Ipv4Addr::LOCALHOST, Ipv4Addr::new(8,8,8,8), Ipv4Addr::new(192,168,1,255)];
        assert_eq!(advertised_ipv4(Some(primary), candidates), vec!["192.168.9.2","10.0.0.2"]);
        let many = (1..=30).map(|i| Ipv4Addr::new(10,0,0,i)).collect();
        assert_eq!(advertised_ipv4(None, many).len(), MAX_ADVERTISED_IPV4);
    }
    #[cfg(windows)]
    #[test]
    fn actual_windows_adapter_discovery_returns_only_bounded_private_literals() {
        let values = local_ipv4_candidates();
        assert!(values.len() <= MAX_ADVERTISED_IPV4);
        for value in &values {
            let ip: Ipv4Addr = value.parse().unwrap();
            assert!(ip.is_private() && !matches!(ip.octets()[3], 0 | 255));
        }
        let unique: std::collections::HashSet<_> = values.iter().collect();
        assert_eq!(values.len(), unique.len());
        // Runner topology may have no usable private NIC. Do not invent one.
        println!("LAN adapter discovery actual private candidate count={}", values.len());
    }
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

    #[tokio::test]
    async fn staged_v5_valid_commit_preserves_ownership_until_authorized() {
        exercise_staged(0).await;
    }

    #[tokio::test]
    async fn staged_v5_rejects_wrong_transcript_commit() {
        exercise_staged(1).await;
    }

    #[tokio::test]
    async fn staged_v5_rejects_commit_after_sharing_stops() {
        exercise_staged(2).await;
    }

    #[tokio::test]
    async fn staged_v5_cancel_does_not_acquire_ownership() {
        exercise_staged(3).await;
    }

    // Actual TCP/WebSocket handshake, not a mock transport. The separate
    // production lease tests cover real video and input ownership transitions.
    async fn exercise_staged(mode: u8) {
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
            control_gate: Arc::new(host::ControlGate::default()),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        let running = config.running.clone();
        let active = config.lan_active.clone();
        let epoch = config.control_gate.video_epoch();
        let initial_epoch = epoch.load(Ordering::SeqCst);
        let listener = TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            accept_authorized(stream, &config).await.map(|(_, key)| key)
        });
        let (mut viewer, _) = connect_async(format!("ws://{address}/direct")).await.unwrap();
        let client = protocol::new_secret();
        viewer.send(Message::Binary(protocol::encrypt(&key, HELLO,
            &transcript(HELLO_V5, &client, &[])).unwrap())).await.unwrap();
        let challenge = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, bytes) = protocol::decrypt(&key, &challenge).unwrap();
        assert_eq!(kind, READY);
        let prefix = transcript(CHALLENGE_V5, &client, &[]);
        let server = bytes.strip_prefix(prefix.as_slice()).unwrap();
        assert_eq!(server.len(), NONCE_LEN);
        viewer.send(Message::Binary(protocol::encrypt(&key, HELLO,
            &transcript(PROOF_V5, &client, server)).unwrap())).await.unwrap();
        let prepared = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        assert_eq!(protocol::decrypt(&key, &prepared).unwrap(),
            (READY, transcript(PREPARED_V5, &client, server)));
        assert!(!active.load(Ordering::SeqCst));
        assert_eq!(epoch.load(Ordering::SeqCst), initial_epoch);
        assert!(!task.is_finished());
        if mode == 3 {
            viewer.close(None).await.unwrap();
        } else {
            if mode == 2 { running.store(false, Ordering::SeqCst); }
            let commit = if mode == 1 {
                transcript(COMMIT_V5, &client, &[0;32])
            } else {
                transcript(COMMIT_V5, &client, server)
            };
            viewer.send(Message::Binary(protocol::encrypt(&key, HELLO, &commit).unwrap()))
                .await.unwrap();
            let reply = tokio::time::timeout(Duration::from_secs(5), viewer.next()).await.unwrap();
            if mode == 0 {
                let packet = reply.unwrap().unwrap().into_data();
                assert_eq!(protocol::decrypt(&key, &packet).unwrap(),
                    (READY, transcript(READY_V5, &client, server)));
            } else {
                assert!(!matches!(reply, Some(Ok(Message::Binary(_)))));
            }
        }
        let result = tokio::time::timeout(Duration::from_secs(6), task).await.unwrap().unwrap();
        if mode == 0 {
            let expected = versioned_business_key(&key, &client, server, HandshakeVersion::V5);
            assert_eq!(result.unwrap(), expected);
            assert_ne!(expected, business_key(&key, &client, server));
        } else {
            assert!(result.is_err());
        }
        assert!(!active.load(Ordering::SeqCst));
        assert_eq!(epoch.load(Ordering::SeqCst), initial_epoch);
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
            control_gate: Arc::new(host::ControlGate::default()),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        let running = config.running.clone();
        let listener = TcpListener::bind(("127.0.0.1",0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok((mut socket, key)) = accept_authorized(stream, &config).await else { return false; };
            let mut sender = protocol::SequenceSender::new(protocol::HOST_DIRECTION);
            let mut guard = protocol::ReplayGuard::new(protocol::VIEW_DIRECTION);
            // Exercise real encrypted WebSocket traffic after READY, without a Relay.
            // This is a transport integration test, not desktop input injection.
            let request = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await.unwrap().unwrap().unwrap().into_data();
            let (kind, payload) = protocol::decrypt(&key, &request).unwrap();
            assert!(guard.accept(&request));
            assert!(!guard.accept(&request));
            assert_eq!(kind, protocol::PING);
            assert_eq!(payload, 42u64.to_be_bytes());
            socket.send(Message::Binary(sender.encrypt(&key, protocol::PONG, &payload).unwrap()))
                .await.unwrap();
            for expected in [br#"{"type":"move","x":0.5,"y":0.5}"#.as_slice(),
                br#"{"type":"clipboard_set","text":"LAN test"}"#.as_slice()] {
                let packet = tokio::time::timeout(Duration::from_secs(5), socket.next())
                    .await.unwrap().unwrap().unwrap().into_data();
                let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
                assert!(guard.accept(&packet));
                assert_eq!(kind, protocol::CONTROL);
                assert_eq!(payload.as_slice(), expected);
            }
            // Encode actual JPEG bytes rather than a fabricated video payload.
            let pixels = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                .encode(&pixels, 2, 2, image::ColorType::Rgb8).unwrap();
            socket.send(Message::Binary(sender.encrypt(&key, protocol::FRAME, &jpeg).unwrap()))
                .await.unwrap();
            socket.send(Message::Binary(sender.encrypt(&key, protocol::CLIPBOARD, b"LAN test").unwrap()))
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
        let mut sender = protocol::SequenceSender::new(protocol::VIEW_DIRECTION);
        let mut guard = protocol::ReplayGuard::new(protocol::HOST_DIRECTION);
        viewer.send(Message::Binary(sender.encrypt(&key, protocol::PING,
            &42u64.to_be_bytes()).unwrap())).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &reply).unwrap();
        assert!(guard.accept(&reply));
        assert_eq!(kind, protocol::PONG);
        assert_eq!(payload, 42u64.to_be_bytes());
        for command in [br#"{"type":"move","x":0.5,"y":0.5}"#.as_slice(),
            br#"{"type":"clipboard_set","text":"LAN test"}"#.as_slice()] {
            viewer.send(Message::Binary(sender.encrypt(&key, protocol::CONTROL, command).unwrap()))
                .await.unwrap();
        }
        let packet = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
        assert!(guard.accept(&packet));
        assert_eq!(kind, protocol::FRAME);
        let decoded = image::load_from_memory_with_format(&payload, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
        let packet = tokio::time::timeout(Duration::from_secs(5), viewer.next())
            .await.unwrap().unwrap().unwrap().into_data();
        let (kind, payload) = protocol::decrypt(&key, &packet).unwrap();
        assert!(guard.accept(&packet));
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

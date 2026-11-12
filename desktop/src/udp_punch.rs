//! Consent-gated, authenticated UDP hole-punch *connectivity test*.
//!
//! Candidate signaling travels through the EXISTING AES-GCM encrypted
//! WebSocket relay. The temporary UDP channel exchanges only encrypted
//! 16-byte challenge/response messages, NEVER video or input.
//! The same bound socket performs STUN and hole-punch probes (unlike V0.8).
use crate::{protocol, stun};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub const SIGNAL: u8 = 10;
pub const HELLO: u8 = 11;
pub const ACK: u8 = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub v: u8,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

pub fn is_request(data: &[u8]) -> bool {
    if data.len() > 256 { return false; }
    serde_json::from_slice::<Signal>(data)
        .map(|s| s.v == 1 && s.kind == "request")
        .unwrap_or(false)
}

fn safe_ipv4(ip: Ipv4Addr) -> bool {
    let a = ip.octets();
    a[0] != 0 && a[0] != 127 && a[0] != 169
        && a[0] < 224 && a != [255, 255, 255, 255]
}

pub fn parse_candidate(data: &[u8], expected_role: &str) -> Option<SocketAddrV4> {
    if data.len() > 256 { return None; }
    let signal: Signal = serde_json::from_slice(data).ok()?;
    if signal.v != 1 || signal.kind != "candidate"
        || signal.role.as_deref() != Some(expected_role) {
        return None;
    }
    let ip = Ipv4Addr::from_str(signal.ip.as_deref()?).ok()?;
    let port = signal.port?;
    if port == 0 || !safe_ipv4(ip) { return None; }
    Some(SocketAddrV4::new(ip, port))
}

pub struct Probe {
    pub peer_tx: Sender<SocketAddrV4>,
    abort: Arc<AtomicBool>,
}

impl Drop for Probe {
    fn drop(&mut self) { self.abort.store(true, Ordering::SeqCst); }
}

pub fn start(
    key: [u8;32],
    relay_url: String,
    session_running: Arc<AtomicBool>,
    log: Sender<String>,
    outbound_signal: UnboundedSender<Vec<u8>>,
) -> Probe {
    let (peer_tx, rx) = mpsc::channel();
    let abort = Arc::new(AtomicBool::new(false));
    let shutdown = abort.clone();
    std::thread::spawn(move || {
        if let Err(error) = run(
            &key, &relay_url, &session_running, &shutdown,
            &log, &outbound_signal, rx,
        ) {
            let _ = log.send(format!("UDP 直连验证失败：{error:#}；继续使用 VPS 中转"));
        }
    });
    Probe { peer_tx, abort }
}

fn run(
    key: &[u8;32],
    relay_url: &str,
    session_running: &AtomicBool,
    abort: &AtomicBool,
    log: &Sender<String>,
    outbound: &UnboundedSender<Vec<u8>>,
    peers: mpsc::Receiver<SocketAddrV4>,
) -> Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0").context("UDP bind blocked")?;
    let (mapped, _) = stun::probe_bound_socket(&sock, relay_url)?;
    sock.set_read_timeout(Some(Duration::from_millis(200)))?;
    let candidate = serde_json::to_vec(&serde_json::json!({
        "v": 1, "type": "candidate", "role": "host",
        "ip": mapped.ip().to_string(), "port": mapped.port()
    }))?;
    outbound.send(candidate.clone()).map_err(|_| anyhow::anyhow!("signal channel closed"))?;
    let _ = log.send("V0.9 UDP 映射已发现，等待 Android 候选地址与加密握手".into());

    let challenge = crate::protocol::new_secret(); // use first 16 bytes as opaque challenge
    let challenge = &challenge[..16];
    let mut remote: Option<SocketAddrV4> = None;
    let mut confirmed_at: Option<Instant> = None;
    let mut last_sent = Instant::now() - Duration::from_millis(300);
    let mut last_candidate = Instant::now();
    let started = Instant::now();
    let mut packet = [0u8; 256];

    while session_running.load(Ordering::SeqCst)
        && !abort.load(Ordering::SeqCst)
        && started.elapsed() < Duration::from_secs(12)
    {
        if confirmed_at.map(|at| at.elapsed() >= Duration::from_secs(1)).unwrap_or(false) {
            return Ok(()); // Grace period for the remote side's final HELLO/ACK.
        }
        while let Ok(peer) = peers.try_recv() {
            remote = Some(peer);
        }
        // Best-effort relay has a bounded queue and may drop signaling under
        // screen-frame backpressure. Retransmit candidate while waiting.
        if remote.is_none() && last_candidate.elapsed() >= Duration::from_millis(1500) {
            let _ = outbound.send(candidate.clone());
            last_candidate = Instant::now();
        }
        if let Some(peer) = remote {
            if last_sent.elapsed() >= Duration::from_millis(300) {
                let bytes = protocol::encrypt(key, HELLO, challenge)?;
                let _ = sock.send_to(&bytes, peer);
                last_sent = Instant::now();
            }
        }
        match sock.recv_from(&mut packet) {
            Ok((n, from)) => {
                if Some(from) != remote.map(SocketAddr::V4) || n > packet.len() {
                    continue; // ignore STUN noise and unauthenticated sources
                }
                if let Ok((kind, plaintext)) = protocol::decrypt(key, &packet[..n]) {
                    if kind == HELLO && plaintext.len() == 16 {
                        let ack = protocol::encrypt(key, ACK, &plaintext)?;
                        let _ = sock.send_to(&ack, from);
                    } else if kind == ACK && plaintext.as_slice() == challenge {
                        if confirmed_at.is_none() {
                            confirmed_at = Some(Instant::now());
                            let _ = log.send(
                                "UDP 加密直连握手成功（仅连通性测试）；视频仍通过 VPS 中转".into()
                            );
                        }
                    }
                }
            }
            Err(err) if matches!(err.kind(), std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::WouldBlock
                | std::io::ErrorKind::ConnectionReset) => {}
            Err(err) => return Err(err).context("UDP receive failed"),
        }
    }
    if confirmed_at.is_none() && session_running.load(Ordering::SeqCst)
        && !abort.load(Ordering::SeqCst)
    {
        let _ = log.send("UDP 双端探测超时（可能为对称 NAT/防火墙）；继续 VPS 中转".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_validation_rejects_unauthorized_or_bad_endpoints() {
        let data = br#"{"v":1,"type":"candidate","role":"viewer","ip":"198.51.100.25","port":42000}"#;
        assert_eq!(parse_candidate(data, "viewer").unwrap().port(), 42000);
        assert!(parse_candidate(data, "host").is_none());
        for ip in ["127.0.0.1","0.0.0.0","224.1.2.3","255.255.255.255","evil.test"] {
            let payload = format!(
                "{{\"v\":1,\"type\":\"candidate\",\"role\":\"viewer\",\"ip\":\"{ip}\",\"port\":1234}}"
            );
            assert!(parse_candidate(payload.as_bytes(),"viewer").is_none());
        }
        assert!(is_request(br#"{"v":1,"type":"request"}"#));
        assert!(!is_request(br#"{"v":2,"type":"request"}"#));
    }

    #[test]
    fn encrypted_udp_challenge_rejects_tampering_and_wrong_secret() {
        let key = protocol::new_secret();
        let challenge = [45u8;16];
        let hello = protocol::encrypt(&key, HELLO, &challenge).unwrap();
        let (kind, message) = protocol::decrypt(&key, &hello).unwrap();
        assert_eq!(kind, HELLO);
        assert_eq!(message, challenge);
        let wrong = protocol::new_secret();
        assert!(protocol::decrypt(&wrong, &hello).is_err());
        let mut changed = hello;
        changed[0] = ACK;
        assert!(protocol::decrypt(&key, &changed).is_err());
    }
}

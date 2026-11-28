use crate::protocol;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc, mpsc::Sender}, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, http::HeaderValue, Message}};

/// Successful encrypted WebSocket video sends only; excludes handshake,
/// metadata, UDP, TLS and TCP overhead. Shared across LAN and WSS workers.
#[derive(Default)]
pub struct TransportVideoStats {
    pub lan_bytes: AtomicU64,
    pub relay_bytes: AtomicU64,
    pub lan_frames: AtomicU64,
    pub relay_frames: AtomicU64,
    pub relay_suppressed_frames: AtomicU64,
    // Socket lifetime, not viewer authentication or delivered frame count.
    pub relay_open_sockets: AtomicUsize,
    pub relay_socket_opens: AtomicU64,
}
struct RelaySocketLease<'a>(&'a TransportVideoStats);
impl<'a> RelaySocketLease<'a> {
    fn open(stats: &'a TransportVideoStats) -> Self {
        stats.relay_open_sockets.fetch_add(1, Ordering::SeqCst);
        stats.relay_socket_opens.fetch_add(1, Ordering::Relaxed);
        Self(stats)
    }
}
impl Drop for RelaySocketLease<'_> {
    fn drop(&mut self) {
        self.0.relay_open_sockets.fetch_sub(1, Ordering::SeqCst);
    }
}
impl TransportVideoStats {
    fn record(&self, lan: bool, bytes: usize) {
        let (total, frames) = if lan { (&self.lan_bytes, &self.lan_frames) }
            else { (&self.relay_bytes, &self.relay_frames) };
        total.fetch_add(bytes as u64, Ordering::Relaxed);
        frames.fetch_add(1, Ordering::Relaxed);
    }
}

// Shared by both transports of one approved Host. No lock is held across
// network awaits. OS input and the LAN ownership transition use the same lock.
#[derive(Default)]
pub struct ControlGate {
    state: std::sync::Mutex<ControlState>,
    // Independent of input permissions. Only a successful LAN lease
    // acquisition or release changes the video transport generation.
    video_epoch: Arc<AtomicU64>,
}
#[derive(Default)]
pub(crate) struct ControlState {
    epoch: u64,
    next_id: u64,
    relay: Option<u64>,
    lan: Option<u64>,
    held: Option<u64>,
}
impl ControlState {
    fn slot(&self, lan: bool) -> Option<u64> {
        if lan { self.lan } else { self.relay }
    }
    fn register(&mut self, lan: bool) -> u64 {
        if let Some(old) = self.slot(lan) { self.release_for(old); }
        self.next_id = self.next_id.checked_add(1).expect("input session id exhausted");
        let id = self.next_id;
        if lan { self.lan = Some(id); } else { self.relay = Some(id); }
        id
    }
    fn current(&self, lan: bool, id: u64, epoch: u64, active: bool) -> bool {
        self.slot(lan) == Some(id) && self.epoch == epoch && lan == active
    }
    fn retire(&mut self, lan: bool, id: u64) {
        self.release_for(id);
        if self.slot(lan) == Some(id) {
            if lan { self.lan = None; } else { self.relay = None; }
        }
    }
    fn release_for(&mut self, id: u64) {
        if self.held == Some(id) {
            #[cfg(windows)]
            let _ = win_input::mouse_button(false, false);
            self.held = None;
        }
    }
    // Call under the gate lock before publishing a transport transition.
    // Every queued command from the previous epoch becomes invalid even if
    // the connection switches away and back before its worker wakes up.
    pub(crate) fn advance(&mut self) {
        if let Some(id) = self.held { self.release_for(id); }
        self.epoch = self.epoch.checked_add(1).expect("input epoch exhausted");
    }
}
impl ControlGate {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn video_epoch(&self) -> Arc<AtomicU64> {
        self.video_epoch.clone()
    }

    // Called by ActiveLan under the same gate lock as the active flag.
    pub(crate) fn advance_video_epoch(&self) {
        self.video_epoch.fetch_update(Ordering::SeqCst, Ordering::SeqCst,
            |value| value.checked_add(1)).expect("video transport epoch exhausted");
    }

    // Permission transitions linearize with native input execution. Even a
    // revoke/regrant faster than the worker's poll invalidates queued work.
    pub fn set_input(&self, allowed: &AtomicBool, value: bool) {
        let mut state = self.lock();
        if allowed.swap(value, Ordering::SeqCst) != value {
            state.advance();
        }
    }

    // Clipboard calls are synchronous and already use this same lock.
    // Do not hold the lock across the subsequent network response.
    pub fn set_clipboard(&self, allowed: &AtomicBool, value: bool) {
        let _state = self.lock();
        allowed.store(value, Ordering::SeqCst);
    }

    pub fn stop(&self, running: &AtomicBool) {
        let mut state = self.lock();
        if running.swap(false, Ordering::SeqCst) {
            state.advance();
        }
    }
}

#[derive(Clone)]
pub struct HostConfig {
    pub server: String,
    pub relay_token: String,
    pub secret: [u8; 32],
    pub running: Arc<AtomicBool>,
    pub allow_input: Arc<AtomicBool>,
    pub allow_clipboard: Arc<AtomicBool>,
    pub monitor_index: Arc<AtomicUsize>,
    pub lan_direct: Arc<AtomicBool>,
    pub lan_active: Arc<AtomicBool>,
    pub control_gate: Arc<ControlGate>,
    pub allow_udp_probe: Arc<AtomicBool>,
    pub allow_udp_preview: Arc<AtomicBool>,
    pub udp_active: Arc<AtomicBool>,
    pub h264_enabled: Arc<AtomicBool>,
    pub target_fps: Arc<AtomicU32>,
    pub sent_frames: Arc<AtomicU64>,
    pub dropped_frames: Arc<AtomicU64>,
    pub transport_video: Arc<TransportVideoStats>,
    pub messages: Sender<String>,
}

pub fn start(config: HostConfig) {
    if config.lan_direct.load(Ordering::SeqCst) {
        crate::lan::spawn(config.clone());
    }
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(error) => {
                let _ = config.messages.send(format!("运行时初始化失败: {error}"));
                config.control_gate.stop(&config.running);
                return;
            }
        };
        if config.relay_token.trim().is_empty() {
            // Direct-only mode: do not keep failing a VPS authorization request.
            let _ = config.messages.send(
                "当前仅启用局域网 P2P 直连，无 VPS 自动回退".into()
            );
            while config.running.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
            }
            return;
        }
        let mut failures = 0u32;
        while config.running.load(Ordering::SeqCst) {
            match runtime.block_on(run(&config)) {
                Ok(()) => {
                    if !config.running.load(Ordering::SeqCst) { break; }
                    let _ = config.messages.send("连接已断开，准备重新连接".into());
                }
                Err(error) => {
                    if !config.running.load(Ordering::SeqCst) { break; }
                    if crate::reconnect::authentication_rejected(&error) {
                        if config.lan_direct.load(Ordering::SeqCst) {
                            let _ = config.messages.send(
                                "VPS 访问口令错误；局域网加密直连仍保持运行".into()
                            );
                            while config.running.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(200));
                            }
                        } else {
                            let _ = config.messages.send(
                                "中转访问口令错误（HTTP 401/403），已停止自动重试".into()
                            );
                        }
                        break;
                    }
                    let _ = config.messages.send(format!("中转连接中断: {error:#}"));
                }
            }
            let retry = crate::reconnect::retry_delay(failures);
            failures = failures.saturating_add(1);
            let _ = config.messages.send(format!("连接恢复中 · {} 秒后重试（点击停止可取消）", retry.as_secs()));
            let started = std::time::Instant::now();
            while config.running.load(Ordering::SeqCst) && started.elapsed() < retry {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        config.control_gate.stop(&config.running);
        let _ = config.messages.send("远程服务已停止".into());
    });
}

pub(crate) async fn run(config: &HostConfig) -> Result<()> {
    let mut endpoint = url::Url::parse(config.server.trim()).context("invalid relay URL")?;
    match endpoint.scheme() {
        "https" => { endpoint.set_scheme("wss").map_err(|_| anyhow!("scheme"))?; }
        "http" => { endpoint.set_scheme("ws").map_err(|_| anyhow!("scheme"))?; }
        "wss" | "ws" => {}
        _ => return Err(anyhow!("only HTTPS/WSS are supported")),
    }
    // Plain ws is permitted for local Docker smoke tests only, never public networks.
    if endpoint.scheme() == "ws" && !matches!(endpoint.host_str(), Some("localhost" | "127.0.0.1")) {
        return Err(anyhow!("public relay requires WSS encryption"));
    }
    endpoint.set_path("/relay");
    endpoint.set_query(None);
    endpoint.query_pairs_mut()
        .append_pair("room", &protocol::room_id(&config.secret))
        .append_pair("role", "host");
    let mut request = endpoint.as_str().into_client_request()?;
    request.headers_mut().insert("Authorization", HeaderValue::from_str(&format!("Bearer {}", config.relay_token))?);
    let (socket, _) = tokio::time::timeout(Duration::from_secs(12), connect_async(request))
        .await.context("relay connection timed out")??;
    let _relay_lease = RelaySocketLease::open(&config.transport_video);
    let _ = config.messages.send("已连接中转服务器 · 等待已授权 Windows / Android 控制端".into());
    serve_session(config, socket, false).await
}

// LAN-only transport liveness, independent of authenticated application RTT.
// A fresh unpredictable payload prevents stale/unsolicited Pong from clearing
// an outstanding probe. This does not grant any business permissions.
const LAN_PING_INTERVAL: Duration = Duration::from_secs(4);
const LAN_PONG_TIMEOUT: Duration = Duration::from_secs(12);

struct LanHeartbeat {
    next: tokio::time::Instant,
    pending: Option<([u8; 8], tokio::time::Instant)>,
}
impl LanHeartbeat {
    fn new(now: tokio::time::Instant) -> Self {
        Self { next: now + LAN_PING_INTERVAL, pending: None }
    }
    fn poll(&mut self, now: tokio::time::Instant) -> Result<Option<[u8; 8]>> {
        if let Some((_, deadline)) = self.pending {
            if now >= deadline { return Err(anyhow!("LAN heartbeat pong timed out")); }
            return Ok(None);
        }
        if now < self.next { return Ok(None); }
        let mut token = [0u8; 8];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut token);
        self.pending = Some((token, now + LAN_PONG_TIMEOUT));
        Ok(Some(token))
    }
    fn pong(&mut self, payload: &[u8], now: tokio::time::Instant) -> bool {
        if let Some((token, deadline)) = self.pending {
            if now < deadline && payload == token.as_slice() {
                self.pending = None;
                self.next = now + LAN_PING_INTERVAL;
                return true;
            }
        }
        false
    }
}

// Once a relay AVC access unit has been discarded, predictive pictures can
// reference data the receiver never got. Resume only at a self-contained IDR.
#[derive(Default)]
struct RelayAvcRecovery {
    observed_epoch: Option<u64>,
    waiting: bool,
    reset_requested: bool,
}
impl RelayAvcRecovery {
    // Even a complete LAN acquire/release between two frame observations
    // invalidates the relay decoder's reference picture history.
    fn observe_epoch(&mut self, epoch: u64) {
        if self.observed_epoch != Some(epoch) {
            if self.observed_epoch.is_some() {
                self.waiting = true;
                self.reset_requested = false;
            }
            self.observed_epoch = Some(epoch);
        }
    }

    fn suppressed(&mut self, kind: u8) {
        if kind == protocol::AVC {
            self.waiting = true;
            self.reset_requested = false;
        }
    }
    fn allow(&mut self, kind: u8, bytes: &[u8], reset: &AtomicBool) -> bool {
        if !self.waiting || kind != protocol::AVC { return true; }
        if self_contained_avc_idr(bytes) {
            self.waiting = false;
            self.reset_requested = false;
            return true;
        }
        if !self.reset_requested {
            reset.store(true, Ordering::Release);
            self.reset_requested = true;
        }
        false
    }
}

// Skip the four-byte wire dimensions header. Require SPS and PPS before
// the first IDR, with no earlier predictive VCL.
fn self_contained_avc_idr(payload: &[u8]) -> bool {
    let Some(bytes) = payload.get(4..) else { return false; };
    if !bytes.starts_with(&[0,0,1]) && !bytes.starts_with(&[0,0,0,1]) {
        return false;
    }
    let (mut sps, mut pps) = (false, false);
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i..].starts_with(&[0,0,1]) {
            let header = bytes[i+3];
            if header & 0x80 != 0 { return false; }
            match header & 31 {
                7 => sps = true,
                8 => pps = true,
                5 => return sps && pps,
                1..=4 => return false,
                _ => {}
            }
            i += 4;
        } else { i += 1; }
    }
    false
}

/// Shared authenticated session pump for VPS WSS and opt-in LAN WebSocket.
/// Only the LAN path performs an additional encrypted HELLO proof before calling this.
pub(crate) async fn serve_session<S>(
    config: &HostConfig,
    socket: tokio_tungstenite::WebSocketStream<S>,
    is_lan_session: bool,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut outgoing, mut incoming) = socket.split();
    // Only authenticated LAN business packets enter this per-connection guard.
    let mut lan_replay = protocol::ReplayGuard::new(protocol::VIEW_DIRECTION);
    let mut lan_sender = protocol::SequenceSender::new(protocol::HOST_DIRECTION);
    let mut encrypt = |kind, bytes: &[u8]| {
        if is_lan_session { lan_sender.encrypt(&config.secret, kind, bytes) }
        else { protocol::encrypt(&config.secret, kind, bytes) }
    };
    #[cfg(windows)]
    let remote_input = InputWorker::start(config.clone(), is_lan_session);
    // Each connection begins in JPEG compatibility mode; AVC is negotiated
    // only by an authenticated viewer with an attached decoding surface.
    let force_jpeg = Arc::new(AtomicBool::new(true));
    let full_hd_requested=Arc::new(AtomicBool::new(false));
    let bitrate = Arc::new(AtomicU32::new(1_800_000));
    let session_fps = Arc::new(AtomicU32::new(config.target_fps.load(Ordering::Relaxed).clamp(6,30)));
    let mut last_adaptation = std::time::Instant::now();
    let mut good_samples = 0u32;
    let actual_encoder=Arc::new(AtomicU32::new(0));
    let video_stats=Arc::new(crate::video::VideoStats::default());
    let encoder_reset = Arc::new(AtomicBool::new(false));
    let video_epoch = config.control_gate.video_epoch();
    let mut relay_avc_recovery = RelayAvcRecovery::default();
    relay_avc_recovery.observe_epoch(video_epoch.load(Ordering::SeqCst));
    let mut frames = crate::video::spawn_frames(
        config.monitor_index.clone(),
        config.h264_enabled.clone(),
        session_fps.clone(),
        bitrate.clone(),
        config.dropped_frames.clone(),
        config.udp_active.clone(),
        force_jpeg.clone(),
        full_hd_requested.clone(),
        actual_encoder.clone(),
        video_stats.clone(),
        encoder_reset.clone(),
        video_epoch.clone(),
    );
    // Refresh bounded candidates while sharing, only in encrypted metadata.
    #[cfg(windows)]
    let mut lan_candidates: Vec<String> = Vec::new();
    #[cfg(windows)]
    let mut lan_candidates_at: Option<std::time::Instant> = None;
    let mut metadata_tick = tokio::time::interval(Duration::from_secs(2));
    // Stop even when capture is idle and the peer sends no messages.
    let mut shutdown_tick = tokio::time::interval(Duration::from_millis(200));
    shutdown_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut heartbeat = is_lan_session.then(|| LanHeartbeat::new(tokio::time::Instant::now()));
    let (udp_signal_tx, mut udp_signal_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let mut udp_probe: Option<crate::udp_punch::Probe> = None;
    loop {
        if !config.running.load(Ordering::SeqCst)
            || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
        tokio::select! {
            _ = shutdown_tick.tick() => {
                if let Some(heartbeat) = &mut heartbeat {
                    if let Some(token) = heartbeat.poll(tokio::time::Instant::now())? {
                        tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Ping(token.to_vec())))
                            .await.context("LAN heartbeat ping send timed out")??;
                    }
                }
            }
            signal = udp_signal_rx.recv() => {
                if let Some(payload) = signal {
                    if config.running.load(Ordering::SeqCst) {
                        let packet = encrypt(crate::udp_punch::SIGNAL, &payload)?;
                        tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                            .await.context("session message send timed out")??;
                    }
                }
            }
            frame = frames.recv() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                // When an authenticated LAN viewer is active, do not waste
                // uplink bandwidth relaying frames to the VPS.
                let Some((frame_epoch, queued_at, (kind, bytes))) = frame else {
                    return Err(anyhow!("screen capture worker exited unexpectedly"));
                };
                // Read ownership and generation together. Do not hold the gate
                // across a socket await; an already-started send cannot be recalled.
                let (lan_active, current_epoch) = {
                    let _gate = config.control_gate.lock();
                    (config.lan_active.load(Ordering::SeqCst),
                        video_epoch.load(Ordering::SeqCst))
                };
                if !is_lan_session {
                    relay_avc_recovery.observe_epoch(current_epoch);
                }
                if !is_lan_session && lan_active {
                    config.transport_video.relay_suppressed_frames.fetch_add(1, Ordering::Relaxed);
                    relay_avc_recovery.suppressed(kind);
                    continue;
                }
                // A queued or still-encoding frame from the previous ownership
                // generation cannot restart video, even if it contains an IDR.
                if frame_epoch != current_epoch || is_lan_session != lan_active {
                    continue;
                }
                if !is_lan_session && config.udp_active.load(Ordering::SeqCst) {
                    relay_avc_recovery.suppressed(kind);
                    continue;
                }
                if !is_lan_session && !relay_avc_recovery.allow(kind, &bytes, &encoder_reset) {
                    continue;
                }
                let queue_wait_us = queued_at.elapsed().as_micros().min(u64::MAX as u128) as u64;
                let send_start=std::time::Instant::now();
                let packet = encrypt(kind, &bytes)?;
                let encrypted_len = packet.len();
                tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                    .await.context("video send timed out")??;
                config.transport_video.record(is_lan_session, encrypted_len);
                video_stats.send.record(send_start);
                video_stats.queue_wait.record_us(queue_wait_us);
                config.sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            _ = metadata_tick.tick() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                #[cfg(windows)]
                {
                    if !is_lan_session && config.lan_direct.load(Ordering::SeqCst) {
                        if lan_candidates_at.map_or(true, |at| at.elapsed() >= Duration::from_secs(10)) {
                            lan_candidates = crate::lan::local_ipv4_candidates();
                            lan_candidates_at = Some(std::time::Instant::now());
                        }
                    } else {
                        lan_candidates.clear();
                        lan_candidates_at = None;
                    }
                    let count = screenshots::Screen::all().map(|screens| screens.len()).unwrap_or(0);
                    let selected = config.monitor_index.load(Ordering::SeqCst).min(count.saturating_sub(1));
                    let capture_timing = video_stats.capture.snapshot();
                    let encode_timing = video_stats.encode.snapshot();
                    let send_timing = video_stats.send.snapshot();
                    let queue_timing = video_stats.queue_wait.snapshot();
                    let info = serde_json::json!({
                        "count": count.min(16), "selected": selected,
                        "lan_ipv4": lan_candidates.first(),
                        "lan_ipv4_candidates": &lan_candidates,
                        "lan_readmission_v5": !is_lan_session && config.lan_direct.load(Ordering::SeqCst),
                        "allow_input": config.allow_input.load(Ordering::SeqCst),
                        "allow_clipboard": config.allow_clipboard.load(Ordering::SeqCst),
                        "video_target_bps": bitrate.load(Ordering::Relaxed),
                        "video_target_fps": session_fps.load(Ordering::Relaxed),
                        "video_full_hd_gpu_negotiation": true,
                        "video_full_hd_requested": full_hd_requested.load(Ordering::SeqCst),
                        "video_encoder": match actual_encoder.load(Ordering::Relaxed) {
                            3 => "Media Foundation D3D11 texture",
                            2 => "Media Foundation hardware",
                            1 => "OpenH264 software",
                            _ => "JPEG",
                        },
                        "video_encoder_hardware": actual_encoder.load(Ordering::Relaxed)>=2,
                        "capture_backend": match video_stats.capture_backend.load(Ordering::Relaxed) {
                            2=>"DXGI", 1=>"screenshots compatibility", _=>"unknown"
                        },
                        "capture_avg_us": capture_timing.average(),
                        "encode_avg_us": encode_timing.average(),
                        "send_avg_us": send_timing.average(),
                        "session_video_timing": {
                            "capture": capture_timing, "encode": encode_timing,
                            "send": send_timing, "queue_wait": queue_timing,
                            "queue_drops": video_stats.queue_drops.load(Ordering::Relaxed),
                            "encode_stages": video_stats.stages.snapshot(),
                        },
                        "fresh_frames": video_stats.fresh_frames.load(Ordering::Relaxed),
                        "reused_frames": video_stats.reused_frames.load(Ordering::Relaxed),
                        "resize_cache_hits": video_stats.resize_cache_hits.load(Ordering::Relaxed),
                        "yuv_cache_hits": video_stats.yuv_cache_hits.load(Ordering::Relaxed),
                        "host_encoded_frames": encode_timing.samples,
                        "host_lan_video_bytes": config.transport_video.lan_bytes.load(Ordering::Relaxed),
                        "host_relay_video_bytes": config.transport_video.relay_bytes.load(Ordering::Relaxed),
                        "host_relay_suppressed_frames": config.transport_video.relay_suppressed_frames.load(Ordering::Relaxed),
                        "host_lan_video_frames": config.transport_video.lan_frames.load(Ordering::Relaxed),
                        "host_relay_video_frames": config.transport_video.relay_frames.load(Ordering::Relaxed),
                        "host_relay_socket_open": config.transport_video.relay_open_sockets.load(Ordering::SeqCst) > 0,
                        "host_relay_socket_opens": config.transport_video.relay_socket_opens.load(Ordering::Relaxed)
                    }).to_string();
                    let packet = encrypt(protocol::MONITORS, info.as_bytes())?;
                    tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                            .await.context("session message send timed out")??;
                }
            }
            next = incoming.next() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                // Never accept two simultaneous remote controllers.
                // Suppress relay business packets, not transport termination.
                // In particular, a socket error must not be discarded in a
                // tight loop while LAN owns the active session.
                if !is_lan_session && config.lan_active.load(Ordering::SeqCst)
                    && matches!(&next, Some(Ok(Message::Binary(_)))) {
                    continue;
                }
                match next {
                    Some(Ok(Message::Binary(packet))) => {
                        if let Ok((kind, bytes)) = protocol::decrypt(&config.secret, &packet) {
                            if is_lan_session && !lan_replay.accept(&packet) { continue; }
                            if kind == crate::udp_punch::SIGNAL && !is_lan_session
                                && config.allow_udp_probe.load(Ordering::SeqCst)
                            {
                                if crate::udp_punch::is_request(&bytes) {
                                    // One bounded test per authenticated VPS session.
                                    // Never start a listener just because an untrusted
                                    // relay packet arrived: SIGNAL is AES-GCM verified.
                                    if udp_probe.is_none() {
                                        let probe = crate::udp_punch::start(
                                            config.secret,
                                            config.server.clone(),
                                            config.running.clone(),
                                            config.messages.clone(),
                                            udp_signal_tx.clone(),
                                            config.allow_udp_preview.clone(),
                                            config.udp_active.clone(),
                                            config.monitor_index.clone(),
                                            config.lan_active.clone(),
                                        );
                                        udp_probe = Some(probe);
                                    }
                                } else if let Some(peer) =
                                    crate::udp_punch::parse_candidate(&bytes, "viewer")
                                {
                                    if let Some(probe) = udp_probe.as_ref() {
                                        let _ = probe.peer_tx.send(peer);
                                    }
                                }
                            } else if kind == protocol::PING && bytes.len() == 8 {
                                // Authenticated application-level ping works on both
                                // LAN direct and WSS relay; no user data is exposed.
                                let pong = encrypt(protocol::PONG, &bytes)?;
                                tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(pong)))
                                    .await.context("application pong send timed out")??;
                            } else if kind == protocol::CONTROL {
                            #[cfg(windows)]
                            {
                                if let Ok(command) = serde_json::from_slice::<Control>(&bytes) {
                                    match command.kind.as_str() {
                                        "video_codec" => {
                                            if let Some((avc,full_hd))=command.codec.as_deref().and_then(crate::video::codec_request) {
                                                if !avc {
                                                    force_jpeg.store(true, Ordering::SeqCst);
                                                    full_hd_requested.store(false, Ordering::SeqCst);
                                                    let _ = config.messages.send(
                                                        "已接收 Viewer 兼容请求：本次连接切换 JPEG 画面".into()
                                                    );
                                                } else if config.h264_enabled.load(Ordering::Relaxed) {
                                                    full_hd_requested.store(full_hd, Ordering::SeqCst);
                                                    force_jpeg.store(false, Ordering::SeqCst);
                                                }
                                            }
                                        }
                                        "video_feedback" if !force_jpeg.load(Ordering::Relaxed)
                                            && config.h264_enabled.load(Ordering::Relaxed) => {
                                            // A bounded controller, not an untrusted requested bitrate.
                                            // Authenticated feedback measures rendered frames and RTT.
                                            if last_adaptation.elapsed() >= Duration::from_secs(4) {
                                                if let (Some(rtt), Some(fps), Some(drops)) =
                                                    (command.rtt_ms, command.rendered_fps, command.decoder_drops) {
                                                    if rtt <= 20_000 && fps.is_finite() && (0.0..=120.0).contains(&fps) && drops <= 10_000 {
                                                        let old = bitrate.load(Ordering::Relaxed);
                                                        let old_fps = session_fps.load(Ordering::Relaxed);
                                                        let (next, next_fps) = adapt_video(old,old_fps,rtt,fps,drops,&mut good_samples);
                                                        bitrate.store(next,Ordering::Relaxed);
                                                        session_fps.store(next_fps,Ordering::Relaxed);
                                                        last_adaptation = std::time::Instant::now();
                                                    }
                                                }
                                            }
                                        }
                                        "monitor" => {
                                            remote_input.with_current(|| {
                                                if let Some(index) = command.index {
                                                    if screenshots::Screen::all()
                                                        .map(|screens| crate::remote_ops::valid_monitor_index(index, screens.len()))
                                                        .unwrap_or(false)
                                                    {
                                                        config.monitor_index.store(index, Ordering::SeqCst);
                                                    }
                                                }
                                            });
                                        }
                                        "clipboard_get" if config.allow_clipboard.load(Ordering::SeqCst) => {
                                            let text = remote_input.with_current(|| {
                                                if !config.allow_clipboard.load(Ordering::SeqCst) { return None; }
                                                let mut clipboard = arboard::Clipboard::new().ok()?;
                                                let text = clipboard.get_text().ok()?;
                                                crate::remote_ops::valid_clipboard_text(&text).then_some(text)
                                            }).flatten();
                                            if let Some(text) = text {
                                                let response = encrypt(protocol::CLIPBOARD, text.as_bytes())?;
                                                tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(response)))
                                                    .await.context("clipboard send timed out")??;
                                            }
                                        }
                                        "clipboard_set" if config.allow_clipboard.load(Ordering::SeqCst) => {
                                            remote_input.with_current(|| {
                                                if !config.allow_clipboard.load(Ordering::SeqCst) { return; }
                                                if let Some(value) = &command.text {
                                                    if crate::remote_ops::valid_clipboard_text(value) {
                                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                            let _ = clipboard.set_text(value.as_str());
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                        _ if config.allow_input.load(Ordering::SeqCst) => {
                                            // Bounded generation-tagged queue. A saturated
                                            // non-move queue fails closed instead of losing an up.
                                            remote_input.enqueue(command).await?;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            }
                        }
                    }
                    Some(Ok(Message::Pong(payload))) => {
                        if let Some(heartbeat) = &mut heartbeat {
                            heartbeat.pong(&payload, tokio::time::Instant::now());
                        }
                    }
                    Some(Ok(Message::Ping(_))) if is_lan_session => {
                        // Tungstenite queues the automatic Pong on read.
                        tokio::time::timeout(Duration::from_secs(8), outgoing.flush())
                            .await.context("LAN websocket pong flush timed out")??;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => return Err(error.into()),
                    _ => {}
                }
            }
        }
    }
    drop(udp_probe);
    #[cfg(windows)]
    drop(remote_input);
    let _ = tokio::time::timeout(Duration::from_secs(2), outgoing.close()).await;
    let _ = config.messages.send("连接已断开".into());
    Ok(())
}

#[derive(Deserialize)]
struct Control {
    #[serde(rename="type")] kind: String,
    x: Option<f64>,
    y: Option<f64>,
    text: Option<String>,
    index: Option<usize>,
    delta: Option<i32>,
    key: Option<String>,
    codec: Option<String>,
    rtt_ms: Option<u64>,
    rendered_fps: Option<f64>,
    decoder_drops: Option<u64>,
}

fn adapt_video(bps:u32,fps:u32,rtt:u64,rendered:f64,drops:u64,good:&mut u32)->(u32,u32) {
    if rtt > 450 || drops > 3 || rendered < (fps as f64)*0.45 {
        *good=0;
        ((bps*3/4).max(400_000),(fps.saturating_sub(3)).max(10.min(fps)))
    } else if rtt < 180 && drops==0 && rendered >= (fps as f64)*0.80 {
        *good+=1;
        if *good>=3 {
            *good=0;
            ((bps+200_000).min(4_000_000),(fps+2).min(30))
        } else {(bps,fps)}
    } else {
        *good=0;
        (bps,fps)
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::adapt_video;
    // Deterministic controller input replay, not a network or video benchmark.
    #[test] fn weak_network_step_and_recovery() {
        let (mut bps, mut fps, mut good) = (1_800_000, 30, 0);
        for _ in 0..12 {
            let next = adapt_video(bps, fps, 600, 4.0, 5, &mut good);
            assert!(next.0 <= bps && next.1 <= fps);
            (bps, fps) = next;
        }
        assert_eq!((bps, fps), (400_000, 10));
        for _ in 0..2 {
            assert_eq!(adapt_video(bps, fps, 90, 10.0, 0, &mut good), (bps, fps));
        }
        // A neutral sample breaks the consecutive recovery streak.
        assert_eq!(adapt_video(bps, fps, 300, 10.0, 0, &mut good), (bps, fps));
        assert_eq!(good, 0);
        for _ in 0..60 {
            let next = adapt_video(bps, fps, 90, fps as f64, 0, &mut good);
            assert!(next.0 >= bps && next.1 >= fps);
            assert!(next.0 <= 4_000_000 && next.1 <= 30);
            (bps, fps) = next;
        }
        assert_eq!((bps, fps), (4_000_000, 30));
    }
    #[test] fn congestion_never_raises_low_configured_fps() {
        for fps in 6..10 {
            let mut good = 2;
            assert_eq!(adapt_video(800_000, fps, 600, 1.0, 5, &mut good), (600_000, fps));
            assert_eq!(good, 0);
        }
    }
    #[test] fn downshift_and_bounded_recovery() {
        let mut good=0;
        assert_eq!(adapt_video(1_800_000,24,600,12.0,5,&mut good),(1_350_000,21));
        assert_eq!(adapt_video(400_000,10,900,1.0,9,&mut good),(400_000,10));
        for _ in 0..2 { assert_eq!(adapt_video(1_800_000,24,90,24.0,0,&mut good),(1_800_000,24)); }
        assert_eq!(adapt_video(1_800_000,24,90,24.0,0,&mut good),(2_000_000,26));
    }
}

#[cfg(windows)]
struct InputWorker {
    config: HostConfig,
    lan: bool,
    id: u64,
    tx: std::sync::mpsc::SyncSender<(u64, Control)>,
}
#[cfg(windows)]
impl InputWorker {
    fn start(config: HostConfig, lan: bool) -> Self {
        use std::sync::mpsc::{self, RecvTimeoutError};
        let (tx, rx) = mpsc::sync_channel::<(u64, Control)>(128);
        let id = config.control_gate.lock().register(lan);
        let worker = config.clone();
        std::thread::spawn(move || {
            let _ = worker.messages.send("Windows 原生 SendInput 输入线程已启动".into());
            let mut last_action = std::time::Instant::now();
            loop {
                {
                    let mut state = worker.control_gate.lock();
                    if state.slot(lan) != Some(id) { break; }
                    if !input_allowed(&worker, lan)
                        || last_action.elapsed() > Duration::from_secs(20) {
                        state.release_for(id);
                    }
                }
                let (epoch, command) = match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(item) => item,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                // Execute under the same lock as LAN activation/deactivation.
                // Old queued work cannot execute after a completed handoff.
                let mut state = worker.control_gate.lock();
                if !input_allowed(&worker, lan) {
                    state.release_for(id);
                    continue;
                }
                if !state.current(lan, id, epoch, worker.lan_active.load(Ordering::SeqCst)) {
                    // Stale work must not release a current-epoch held button.
                    continue;
                }
                last_action = std::time::Instant::now();
                let held = state.held == Some(id);
                let result = match command.kind.as_str() {
                    "down" if valid_pointer(&command) => {
                        if !held {
                            let result = handle_control(&command, worker.monitor_index.load(Ordering::SeqCst));
                            if result.is_ok() { state.held = Some(id); }
                            result
                        } else { Ok(()) }
                    }
                    "up" => {
                        let result = if held { win_input::mouse_button(false, false) } else { Ok(()) };
                        if result.is_ok() && held { state.held = None; }
                        result
                    }
                    "right_click" | "move" if valid_pointer(&command) =>
                        handle_control(&command, worker.monitor_index.load(Ordering::SeqCst)),
                    "text" | "key" | "scroll" =>
                        handle_control(&command, worker.monitor_index.load(Ordering::SeqCst)),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    let _ = worker.messages.send(format!("远程输入执行失败：{error}"));
                }
            }
            worker.control_gate.lock().retire(lan, id);
        });
        Self { config, lan, id, tx }
    }

    fn with_current<T>(&self, action: impl FnOnce() -> T) -> Option<T> {
        let state = self.config.control_gate.lock();
        if !session_allowed(&self.config, self.lan)
            || !state.current(self.lan, self.id, state.epoch, self.config.lan_active.load(Ordering::SeqCst)) {
            return None;
        }
        // Keep the guard live across the synchronous side effect.
        let result = action();
        drop(state);
        Some(result)
    }

    async fn enqueue(&self, command: Control) -> Result<()> {
        use std::sync::mpsc::TrySendError;
        let epoch = {
            let state = self.config.control_gate.lock();
            if !input_allowed(&self.config, self.lan)
                || !state.current(self.lan, self.id, state.epoch, self.config.lan_active.load(Ordering::SeqCst)) {
                return Ok(());
            }
            state.epoch
        };
        let is_move = command.kind == "move";
        let mut item = (epoch, command);
        let started = std::time::Instant::now();
        loop {
            match self.tx.try_send(item) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(_)) if is_move => return Ok(()),
                Err(TrySendError::Full(pending)) => {
                    item = pending;
                    {
                        let state = self.config.control_gate.lock();
                        if !input_allowed(&self.config, self.lan)
                            || !state.current(self.lan, self.id, epoch, self.config.lan_active.load(Ordering::SeqCst)) {
                            return Ok(());
                        }
                    }
                    if started.elapsed() >= Duration::from_secs(1) {
                        return Err(anyhow!("remote input queue saturated"));
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(TrySendError::Disconnected(_)) => return Err(anyhow!("remote input worker stopped")),
            }
        }
    }
}
#[cfg(windows)]
impl Drop for InputWorker {
    fn drop(&mut self) {
        // Invalidate immediately, even when serve_session exits via error or
        // cancellation. The old worker cannot release a newer owner's mouse.
        self.config.control_gate.lock().retire(self.lan, self.id);
    }
}
#[cfg(windows)]
fn session_allowed(config: &HostConfig, lan: bool) -> bool {
    config.running.load(Ordering::SeqCst)
        && (!lan || config.lan_direct.load(Ordering::SeqCst))
}
#[cfg(windows)]
fn input_allowed(config: &HostConfig, lan: bool) -> bool {
    session_allowed(config, lan) && config.allow_input.load(Ordering::SeqCst)
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;
    #[test]
    fn matching_pong_only_and_deadline_not_extended_by_unrelated_traffic() {
        let start = tokio::time::Instant::now();
        let mut h = LanHeartbeat::new(start);
        assert!(h.poll(start).unwrap().is_none());
        let sent = start + LAN_PING_INTERVAL;
        let token = h.poll(sent).unwrap().unwrap();
        let mut wrong = token;
        wrong[0] ^= 1;
        assert!(!h.pong(&wrong, sent + Duration::from_secs(1)));
        assert!(h.poll(sent + Duration::from_secs(11)).unwrap().is_none());
        assert!(!h.pong(&token, sent + LAN_PONG_TIMEOUT));
        assert!(h.poll(sent + LAN_PONG_TIMEOUT).is_err());
    }
    #[test]
    fn healthy_pong_rearms_and_duplicate_does_not_refresh() {
        let start = tokio::time::Instant::now();
        let mut h = LanHeartbeat::new(start);
        let sent = start + LAN_PING_INTERVAL;
        let token = h.poll(sent).unwrap().unwrap();
        let received = sent + Duration::from_secs(1);
        assert!(h.pong(&token, received));
        assert!(!h.pong(&token, received + Duration::from_secs(2)));
        assert!(h.poll(received + Duration::from_secs(3)).unwrap().is_none());
        assert!(h.poll(received + LAN_PING_INTERVAL).unwrap().is_some());
    }
}

#[cfg(test)]
mod input_gate_tests {
    use super::*;
    #[test]
    fn rapid_permission_revoke_and_regrant_invalidates_queued_epoch() {
        let gate = ControlGate::default();
        let allowed = AtomicBool::new(true);
        let running = AtomicBool::new(true);
        let (id, queued) = {
            let mut state = gate.lock();
            let id = state.register(false);
            (id, state.epoch)
        };
        gate.set_input(&allowed, true);
        assert!(gate.lock().current(false, id, queued, false));
        gate.set_input(&allowed, false);
        gate.set_input(&allowed, true);
        let fresh = gate.lock().epoch;
        assert!(allowed.load(Ordering::SeqCst));
        assert!(!gate.lock().current(false, id, queued, false));
        assert!(gate.lock().current(false, id, fresh, false));
        gate.stop(&running);
        assert!(!running.load(Ordering::SeqCst));
        assert!(!gate.lock().current(false, id, fresh, false));
    }
    #[test]
    fn clipboard_permission_does_not_invalidate_independent_input_epoch() {
        let gate = ControlGate::default();
        let clipboard = AtomicBool::new(false);
        let (id, epoch) = {
            let mut state = gate.lock();
            let id = state.register(true);
            (id, state.epoch)
        };
        gate.set_clipboard(&clipboard, true);
        gate.set_clipboard(&clipboard, false);
        assert!(!clipboard.load(Ordering::SeqCst));
        assert!(gate.lock().current(true, id, epoch, true));
    }
    #[test]
    fn old_queue_epochs_and_retired_sessions_cannot_execute() {
        let gate = ControlGate::default();
        let mut state = gate.lock();
        let relay = state.register(false);
        let original = state.epoch;
        assert!(state.current(false, relay, original, false));
        state.advance();
        let lan = state.register(true);
        let direct = state.epoch;
        assert!(!state.current(false, relay, original, true));
        assert!(!state.current(false, relay, direct, true));
        assert!(state.current(true, lan, direct, true));
        state.retire(true, lan);
        state.advance();
        let resumed = state.epoch;
        assert!(!state.current(false, relay, original, false));
        assert!(!state.current(true, lan, direct, false));
        assert!(state.current(false, relay, resumed, false));
        let replacement = state.register(false);
        state.retire(false, relay);
        assert!(!state.current(false, relay, resumed, false));
        assert!(state.current(false, replacement, resumed, false));
    }
    #[test]
    fn relay_reconnect_does_not_invalidate_current_lan_queue() {
        let gate = ControlGate::default();
        let mut state = gate.lock();
        let old = state.register(false);
        state.advance();
        let lan = state.register(true);
        let epoch = state.epoch;
        let new = state.register(false);
        state.retire(false, old);
        assert!(state.current(true, lan, epoch, true));
        assert!(!state.current(false, new, epoch, true));
    }
}

#[cfg(windows)]
fn valid_pointer(command: &Control) -> bool {
    match (command.x, command.y) {
        (Some(x), Some(y)) => x.is_finite() && y.is_finite()
            && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
        _ => false,
    }
}

#[cfg(windows)]
fn handle_control(command: &Control, index: usize) -> Result<(), String> {
    if command.kind == "text" {
        if let Some(value) = &command.text {
            if value.chars().count() <= 250 { win_input::unicode_text(value)?; }
        }
        return Ok(());
    }
    if command.kind == "key" {
        if let Some(name) = command.key.as_deref() {
            if crate::remote_ops::valid_remote_key(name) {
                win_input::key(name)?;
            }
        }
        return Ok(());
    }
    if command.kind == "scroll" {
        if let Some(delta) = command.delta {
            if (-10..=10).contains(&delta) && delta != 0 {
                win_input::wheel(delta)?;
            }
        }
        return Ok(());
    }
    if !matches!(command.kind.as_str(), "down" | "up" | "move" | "right_click") {
        return Ok(());
    }
    let (Some(x), Some(y)) = (command.x, command.y) else { return Ok(()); };
    if !x.is_finite() || !y.is_finite()
        || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Ok(());
    }
    let screens = screenshots::Screen::all().map_err(|e| format!("读取显示器失败：{e}"))?;
    let screen = screens.get(index).ok_or_else(|| "目标显示器不存在".to_string())?;
    let info = &screen.display_info;
    let px = info.x + (x * (info.width.saturating_sub(1) as f64)) as i32;
    let py = info.y + (y * (info.height.saturating_sub(1) as f64)) as i32;
    win_input::move_to(px, py)?;
    match command.kind.as_str() {
        "down" => win_input::mouse_button(true, false),
        "up" => win_input::mouse_button(false, false),
        "right_click" => win_input::right_click(),
        _ => Ok(()),
    }
}

#[cfg(windows)]
mod win_input {
    use std::mem;

    const INPUT_MOUSE: u32 = 0;
    const INPUT_KEYBOARD: u32 = 1;
    const MOUSEEVENTF_MOVE: u32 = 0x0001;
    const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
    const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
    const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
    const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
    const MOUSEEVENTF_WHEEL: u32 = 0x0800;
    const MOUSEEVENTF_VIRTUALDESK: u32 = 0x4000;
    const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;
    const KEYEVENTF_KEYUP: u32 = 0x0002;
    const KEYEVENTF_UNICODE: u32 = 0x0004;
    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    const SM_CXVIRTUALSCREEN: i32 = 78;
    const SM_CYVIRTUALSCREEN: i32 = 79;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MouseInput {
        dx: i32, dy: i32, mouse_data: u32, flags: u32, time: u32, extra_info: usize,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyboardInput {
        vk: u16, scan: u16, flags: u32, time: u32, extra_info: usize,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct HardwareInput { message: u32, param_l: u16, param_h: u16 }
    #[repr(C)]
    union InputData { mouse: MouseInput, keyboard: KeyboardInput, hardware: HardwareInput }
    #[repr(C)]
    struct Input { kind: u32, data: InputData }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
        fn GetSystemMetrics(index: i32) -> i32;
        fn GetLastError() -> u32;
    }

    fn submit(input: Input) -> Result<(), String> {
        let sent = unsafe { SendInput(1, &input, mem::size_of::<Input>() as i32) };
        if sent == 1 { Ok(()) } else {
            let error = unsafe { GetLastError() };
            Err(format!("SendInput 未注入事件（返回 {sent}，Win32 错误 {error}）。若目标程序以管理员身份运行，请以相同权限启动 P2P Remote"))
        }
    }

    fn mouse(flags: u32, data: u32) -> Result<(), String> {
        submit(Input { kind: INPUT_MOUSE, data: InputData { mouse: MouseInput {
            dx: 0, dy: 0, mouse_data: data, flags, time: 0, extra_info: 0,
        }}})
    }

    pub fn move_to(x: i32, y: i32) -> Result<(), String> {
        let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
        let dx = (((x - left) as i64 * 65535) / (width - 1).max(1) as i64).clamp(0, 65535) as i32;
        let dy = (((y - top) as i64 * 65535) / (height - 1).max(1) as i64).clamp(0, 65535) as i32;
        submit(Input { kind: INPUT_MOUSE, data: InputData { mouse: MouseInput {
            dx, dy, mouse_data: 0,
            flags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            time: 0, extra_info: 0,
        }}})
    }

    pub fn mouse_button(down: bool, right: bool) -> Result<(), String> {
        let flags = match (down, right) {
            (true, false) => MOUSEEVENTF_LEFTDOWN,
            (false, false) => MOUSEEVENTF_LEFTUP,
            (true, true) => MOUSEEVENTF_RIGHTDOWN,
            (false, true) => MOUSEEVENTF_RIGHTUP,
        };
        mouse(flags, 0)
    }

    pub fn right_click() -> Result<(), String> {
        mouse_button(true, true)?;
        mouse_button(false, true)
    }

    pub fn wheel(delta: i32) -> Result<(), String> {
        mouse(MOUSEEVENTF_WHEEL, (delta * 120) as u32)
    }

    fn vk(name: &str) -> Option<u16> {
        Some(match name {
            "enter" => 0x0D, "backspace" => 0x08, "tab" => 0x09,
            "escape" => 0x1B, "delete" => 0x2E, "up" => 0x26,
            "down" => 0x28, "left" => 0x25, "right" => 0x27,
            "home" => 0x24, "end" => 0x23, "page_up" => 0x21,
            "page_down" => 0x22, "space" => 0x20,
            _ => return None,
        })
    }

    fn keyboard(vk: u16, flags: u32) -> Result<(), String> {
        submit(Input { kind: INPUT_KEYBOARD, data: InputData { keyboard: KeyboardInput {
            vk, scan: 0, flags, time: 0, extra_info: 0,
        }}})
    }

    pub fn key(name: &str) -> Result<(), String> {
        let combo = match name {
            "ctrl_a" => Some(0x41), "ctrl_c" => Some(0x43),
            "ctrl_v" => Some(0x56), "ctrl_x" => Some(0x58),
            "ctrl_z" => Some(0x5A), "ctrl_y" => Some(0x59),
            _ => None,
        };
        if let Some(letter) = combo {
            keyboard(0x11, 0)?;
            let result = keyboard(letter, 0).and_then(|_| keyboard(letter, KEYEVENTF_KEYUP));
            let release = keyboard(0x11, KEYEVENTF_KEYUP);
            result?;
            return release;
        }
        if let Some(code) = vk(name) {
            keyboard(code, 0)?;
            keyboard(code, KEYEVENTF_KEYUP)?;
        }
        Ok(())
    }

    pub fn unicode_text(value: &str) -> Result<(), String> {
        for unit in value.encode_utf16() {
            submit(Input { kind: INPUT_KEYBOARD, data: InputData { keyboard: KeyboardInput {
                vk: 0, scan: unit, flags: KEYEVENTF_UNICODE, time: 0, extra_info: 0,
            }}})?;
            submit(Input { kind: INPUT_KEYBOARD, data: InputData { keyboard: KeyboardInput {
                vk: 0, scan: unit, flags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, time: 0, extra_info: 0,
            }}})?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod transport_video_tests {
    use super::*;
    #[test]
    fn relay_socket_lease_counts_open_and_drop_without_resetting_history() {
        let stats = TransportVideoStats::default();
        assert_eq!(stats.relay_open_sockets.load(Ordering::SeqCst), 0);
        {
            let _a = RelaySocketLease::open(&stats);
            let b = RelaySocketLease::open(&stats);
            assert_eq!(stats.relay_open_sockets.load(Ordering::SeqCst), 2);
            drop(b);
            assert_eq!(stats.relay_open_sockets.load(Ordering::SeqCst), 1);
        }
        assert_eq!(stats.relay_open_sockets.load(Ordering::SeqCst), 0);
        assert_eq!(stats.relay_socket_opens.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn relay_recovery_waits_for_self_contained_idr_and_requests_one_reset() {
        let mut gate = RelayAvcRecovery::default();
        let reset = AtomicBool::new(false);
        let p = [0,2,0,2,0,0,1,0x61,0x80];
        let idr_only = [0,2,0,2,0,0,1,0x65,0x80];
        let key = [0,2,0,2,0,0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x65,0x80];
        assert!(gate.allow(protocol::AVC, &p, &reset));
        gate.suppressed(protocol::FRAME);
        assert!(gate.allow(protocol::AVC, &p, &reset));
        gate.suppressed(protocol::AVC);
        assert!(gate.allow(protocol::FRAME, &[], &reset));
        assert!(!gate.allow(protocol::AVC, &p, &reset));
        assert!(reset.swap(false, Ordering::AcqRel));
        assert!(!gate.allow(protocol::AVC, &idr_only, &reset));
        assert!(!gate.allow(protocol::AVC, &[0; 4], &reset));
        assert!(!reset.load(Ordering::Acquire));
        let mut wrong_order = p.to_vec();
        wrong_order.extend_from_slice(&key[4..]);
        assert!(!gate.allow(protocol::AVC, &wrong_order, &reset));
        assert!(gate.allow(protocol::AVC, &key, &reset));
        assert!(gate.allow(protocol::AVC, &p, &reset));
        gate.suppressed(protocol::AVC);
        assert!(!gate.allow(protocol::AVC, &p, &reset));
        assert!(reset.load(Ordering::Acquire));
    }

    #[test]
    fn rapid_transport_epoch_change_without_suppressed_packet_requires_idr() {
        let mut gate = RelayAvcRecovery::default();
        let reset = AtomicBool::new(false);
        let p = [0,2,0,2,0,0,1,0x61,0x80];
        let key = [0,2,0,2,0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x65,0x80];
        gate.observe_epoch(0);
        assert!(gate.allow(protocol::AVC, &p, &reset));
        // No suppressed() call: both LAN transitions occurred between samples.
        gate.observe_epoch(2);
        assert!(!gate.allow(protocol::AVC, &p, &reset));
        assert!(reset.swap(false, Ordering::AcqRel));
        gate.observe_epoch(2);
        assert!(!gate.allow(protocol::AVC, &p, &reset));
        assert!(!reset.load(Ordering::Acquire));
        assert!(gate.allow(protocol::AVC, &key, &reset));
        gate.observe_epoch(2);
        assert!(gate.allow(protocol::AVC, &p, &reset));
        gate.observe_epoch(4);
        assert!(!gate.allow(protocol::AVC, &p, &reset));
        assert!(reset.load(Ordering::Acquire));
    }

    #[test]
    fn video_counters_separate_lan_and_relay() {
        let stats = TransportVideoStats::default();
        stats.record(false, 100);
        stats.record(true, 200);
        stats.record(true, 300);
        assert_eq!(stats.relay_bytes.load(Ordering::Relaxed), 100);
        assert_eq!(stats.lan_bytes.load(Ordering::Relaxed), 500);
        assert_eq!(stats.relay_frames.load(Ordering::Relaxed), 1);
        assert_eq!(stats.lan_frames.load(Ordering::Relaxed), 2);
    }
}

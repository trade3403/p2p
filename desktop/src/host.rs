use crate::protocol;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc, mpsc::Sender}, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, http::HeaderValue, Message}};

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
    pub allow_udp_probe: Arc<AtomicBool>,
    pub allow_udp_preview: Arc<AtomicBool>,
    pub udp_active: Arc<AtomicBool>,
    pub h264_enabled: Arc<AtomicBool>,
    pub target_fps: Arc<AtomicU32>,
    pub sent_frames: Arc<AtomicU64>,
    pub dropped_frames: Arc<AtomicU64>,
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
                config.running.store(false, Ordering::SeqCst);
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
        config.running.store(false, Ordering::SeqCst);
        let _ = config.messages.send("远程服务已停止".into());
    });
}

async fn run(config: &HostConfig) -> Result<()> {
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
    let _ = config.messages.send("已连接中转服务器 · 等待已授权 Windows / Android 控制端".into());
    serve_session(config, socket, false).await
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
    let remote_input = start_input_worker(
        config.allow_input.clone(), config.monitor_index.clone(),
        config.messages.clone()
    );
    // Each connection begins in JPEG compatibility mode; AVC is negotiated
    // only by an authenticated viewer with an attached decoding surface.
    let force_jpeg = Arc::new(AtomicBool::new(true));
    let bitrate = Arc::new(AtomicU32::new(1_800_000));
    let session_fps = Arc::new(AtomicU32::new(config.target_fps.load(Ordering::Relaxed).clamp(6,30)));
    let mut last_adaptation = std::time::Instant::now();
    let mut good_samples = 0u32;
    let actual_encoder=Arc::new(AtomicU32::new(0));
    let video_stats=Arc::new(crate::video::VideoStats::default());
    let mut frames = crate::video::spawn_frames(
        config.monitor_index.clone(),
        config.h264_enabled.clone(),
        session_fps.clone(),
        bitrate.clone(),
        config.dropped_frames.clone(),
        config.udp_active.clone(),
        force_jpeg.clone(),
        actual_encoder.clone(),
        video_stats.clone(),
    );
    // UDP connect selects a local interface without sending a datagram.
    // The candidate is advertised only inside session-encrypted metadata.
    #[cfg(windows)]
    let lan_ipv4 = if !is_lan_session && config.lan_direct.load(Ordering::SeqCst) {
        std::net::UdpSocket::bind("0.0.0.0:0").ok().and_then(|socket| {
            socket.connect("1.1.1.1:443").ok()?;
            match socket.local_addr().ok()?.ip() {
                std::net::IpAddr::V4(ip) if ip.is_private() => Some(ip.to_string()),
                _ => None,
            }
        })
    } else { None };
    let mut metadata_tick = tokio::time::interval(Duration::from_secs(2));
    let (udp_signal_tx, mut udp_signal_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let mut udp_probe: Option<crate::udp_punch::Probe> = None;
    loop {
        if !config.running.load(Ordering::SeqCst)
            || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
        tokio::select! {
            signal = udp_signal_rx.recv() => {
                if let Some(payload) = signal {
                    if config.running.load(Ordering::SeqCst) {
                        let packet = encrypt(crate::udp_punch::SIGNAL, &payload)?;
                        outgoing.send(Message::Binary(packet)).await?;
                    }
                }
            }
            frame = frames.recv() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                // When an authenticated LAN viewer is active, do not waste
                // uplink bandwidth relaying frames to the VPS.
                if !is_lan_session && (config.lan_active.load(Ordering::SeqCst)
                    || config.udp_active.load(Ordering::SeqCst)) { continue; }
                let Some((kind, bytes)) = frame else {
                    return Err(anyhow!("screen capture worker exited unexpectedly"));
                };
                let send_start=std::time::Instant::now();
                let packet = encrypt(kind, &bytes)?;
                tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                    .await.context("video send timed out")??;
                crate::video::VideoStats::record(&video_stats.send_total_us,&video_stats.send_samples,send_start);
                config.sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            _ = metadata_tick.tick() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                #[cfg(windows)]
                {
                    let count = screenshots::Screen::all().map(|screens| screens.len()).unwrap_or(0);
                    let selected = config.monitor_index.load(Ordering::SeqCst).min(count.saturating_sub(1));
                    let info = serde_json::json!({
                        "count": count.min(16), "selected": selected,
                        "lan_ipv4": lan_ipv4.as_deref(),
                        "allow_input": config.allow_input.load(Ordering::SeqCst),
                        "allow_clipboard": config.allow_clipboard.load(Ordering::SeqCst),
                        "video_target_bps": bitrate.load(Ordering::Relaxed),
                        "video_target_fps": session_fps.load(Ordering::Relaxed),
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
                        "capture_avg_us": crate::video::VideoStats::average(&video_stats.capture_total_us,&video_stats.capture_samples),
                        "encode_avg_us": crate::video::VideoStats::average(&video_stats.encode_total_us,&video_stats.encode_samples),
                        "send_avg_us": crate::video::VideoStats::average(&video_stats.send_total_us,&video_stats.send_samples),
                        "fresh_frames": video_stats.fresh_frames.load(Ordering::Relaxed),
                        "reused_frames": video_stats.reused_frames.load(Ordering::Relaxed),
                        "resize_cache_hits": video_stats.resize_cache_hits.load(Ordering::Relaxed),
                        "yuv_cache_hits": video_stats.yuv_cache_hits.load(Ordering::Relaxed),
                        "host_encoded_frames": video_stats.encode_samples.load(Ordering::Relaxed)
                    }).to_string();
                    let packet = encrypt(protocol::MONITORS, info.as_bytes())?;
                    outgoing.send(Message::Binary(packet)).await?;
                }
            }
            next = incoming.next() => {
                if !config.running.load(Ordering::SeqCst)
                    || (is_lan_session && !config.lan_direct.load(Ordering::SeqCst)) { break; }
                // Never accept two simultaneous remote controllers.
                if !is_lan_session && config.lan_active.load(Ordering::SeqCst) {
                    if matches!(next, Some(Ok(Message::Close(_))) | None) { break; }
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
                                outgoing.send(Message::Binary(pong)).await?;
                            } else if kind == protocol::CONTROL {
                            #[cfg(windows)]
                            {
                                if let Ok(command) = serde_json::from_slice::<Control>(&bytes) {
                                    match command.kind.as_str() {
                                        "video_codec" => {
                                            if command.codec.as_deref() == Some("jpeg") {
                                                force_jpeg.store(true, Ordering::SeqCst);
                                                let _ = config.messages.send(
                                                    "已接收 Viewer 兼容请求：本次连接切换 JPEG 画面".into()
                                                );
                                            } else if command.codec.as_deref() == Some("h264")
                                                && config.h264_enabled.load(Ordering::Relaxed) {
                                                force_jpeg.store(false, Ordering::SeqCst);
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
                                            if let Some(index) = command.index {
                                                if screenshots::Screen::all()
                                                    .map(|screens| crate::remote_ops::valid_monitor_index(index, screens.len()))
                                                    .unwrap_or(false)
                                                {
                                                    config.monitor_index.store(index, Ordering::SeqCst);
                                                }
                                            }
                                        }
                                        "clipboard_get" if config.allow_clipboard.load(Ordering::SeqCst) => {
                                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                if let Ok(text) = clipboard.get_text() {
                                                    if crate::remote_ops::valid_clipboard_text(&text) {
                                                        let response = encrypt(protocol::CLIPBOARD, text.as_bytes())?;
                                                        outgoing.send(Message::Binary(response)).await?;
                                                    }
                                                }
                                            }
                                        }
                                        "clipboard_set" if config.allow_clipboard.load(Ordering::SeqCst) => {
                                            if let Some(value) = &command.text {
                                                if crate::remote_ops::valid_clipboard_text(value) {
                                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                                        let _ = clipboard.set_text(value.as_str());
                                                    }
                                                }
                                            }
                                        }
                                        _ if config.allow_input.load(Ordering::SeqCst) => {
                                            // Preserve mouse-button state on one dedicated OS
                                            // input thread instead of rebuilding Enigo per packet.
                                            // The queue is bounded to avoid accumulating stale moves.
                                            if command.kind == "move" {
                                                let _ = remote_input.try_send(command);
                                            } else {
                                                let _ = remote_input.send(command);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => return Err(error.into()),
                    _ => {}
                }
            }
        }
    }
    drop(udp_probe);
    let _ = outgoing.close().await;
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
fn start_input_worker(
    allowed: Arc<AtomicBool>,
    monitor: Arc<AtomicUsize>,
    messages: Sender<String>,
) -> std::sync::mpsc::SyncSender<Control> {
    use std::sync::mpsc::{self, RecvTimeoutError};
    let (tx, rx) = mpsc::sync_channel::<Control>(128);
    std::thread::spawn(move || {
        let _ = messages.send("Windows 原生 SendInput 输入线程已启动".into());
        let mut held = false;
        let mut last_action = std::time::Instant::now();
        loop {
            if !allowed.load(Ordering::SeqCst) && held {
                let _ = win_input::mouse_button(false, false);
                held = false;
            }
            let command = match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => {
                    if held && last_action.elapsed() > Duration::from_secs(20) {
                        let _ = win_input::mouse_button(false, false);
                        held = false;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if !allowed.load(Ordering::SeqCst) { continue; }
            last_action = std::time::Instant::now();
            let result = match command.kind.as_str() {
                "down" if valid_pointer(&command) => {
                    if !held {
                        let result = handle_control(&command, monitor.load(Ordering::SeqCst));
                        if result.is_ok() { held = true; }
                        result
                    } else { Ok(()) }
                }
                "up" => {
                    let result = if held { win_input::mouse_button(false, false) } else { Ok(()) };
                    held = false;
                    result
                }
                "right_click" | "move" if valid_pointer(&command) =>
                    handle_control(&command, monitor.load(Ordering::SeqCst)),
                "text" | "key" | "scroll" =>
                    handle_control(&command, monitor.load(Ordering::SeqCst)),
                _ => Ok(()),
            };
            if let Err(error) = result {
                let _ = messages.send(format!("远程输入执行失败：{error}"));
            }
        }
        if held { let _ = win_input::mouse_button(false, false); }
    });
    tx
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

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
    let _ = config.messages.send("已连接中转服务器 · 等待已授权手机连接".into());
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
    #[cfg(windows)]
    let remote_input = start_input_worker(
        config.allow_input.clone(), config.monitor_index.clone(),
        config.messages.clone()
    );
    let force_jpeg = Arc::new(AtomicBool::new(false));
    let mut frames = crate::video::spawn_frames(
        config.monitor_index.clone(),
        config.h264_enabled.clone(),
        config.target_fps.clone(),
        config.dropped_frames.clone(),
        config.udp_active.clone(),
        force_jpeg.clone(),
    );
    let mut metadata_tick = tokio::time::interval(Duration::from_secs(5));
    let (udp_signal_tx, mut udp_signal_rx) =
        tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let mut udp_probe: Option<crate::udp_punch::Probe> = None;
    loop {
        tokio::select! {
            signal = udp_signal_rx.recv() => {
                if let Some(payload) = signal {
                    if config.running.load(Ordering::SeqCst) {
                        let packet = protocol::encrypt(
                            &config.secret, crate::udp_punch::SIGNAL, &payload
                        )?;
                        outgoing.send(Message::Binary(packet)).await?;
                    }
                }
            }
            frame = frames.recv() => {
                if !config.running.load(Ordering::SeqCst) { break; }
                // When an authenticated LAN viewer is active, do not waste
                // uplink bandwidth relaying frames to the VPS.
                if !is_lan_session && (config.lan_active.load(Ordering::SeqCst)
                    || config.udp_active.load(Ordering::SeqCst)) { continue; }
                let Some((kind, bytes)) = frame else {
                    return Err(anyhow!("screen capture worker exited unexpectedly"));
                };
                let packet = protocol::encrypt(&config.secret, kind, &bytes)?;
                tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                    .await.context("video send timed out")??;
                config.sent_frames.fetch_add(1, Ordering::Relaxed);
            }
            _ = metadata_tick.tick() => {
                #[cfg(windows)]
                {
                    let count = screenshots::Screen::all().map(|screens| screens.len()).unwrap_or(0);
                    let selected = config.monitor_index.load(Ordering::SeqCst).min(count.saturating_sub(1));
                    let info = serde_json::json!({
                        "count": count.min(16), "selected": selected,
                        "allow_input": config.allow_input.load(Ordering::SeqCst),
                        "allow_clipboard": config.allow_clipboard.load(Ordering::SeqCst)
                    }).to_string();
                    let packet = protocol::encrypt(&config.secret, protocol::MONITORS, info.as_bytes())?;
                    outgoing.send(Message::Binary(packet)).await?;
                }
            }
            next = incoming.next() => {
                // Never accept two simultaneous remote controllers.
                if !is_lan_session && config.lan_active.load(Ordering::SeqCst) {
                    if matches!(next, Some(Ok(Message::Close(_))) | None) { break; }
                    continue;
                }
                match next {
                    Some(Ok(Message::Binary(packet))) => {
                        if let Ok((kind, bytes)) = protocol::decrypt(&config.secret, &packet) {
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
                                let pong = protocol::encrypt(&config.secret, protocol::PONG, &bytes)?;
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
                                                    "已接收 Windows Viewer 兼容请求：本次连接切换 JPEG 画面".into()
                                                );
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
                                                        let response = protocol::encrypt(
                                                            &config.secret, protocol::CLIPBOARD, text.as_bytes()
                                                        )?;
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
}

#[cfg(windows)]
fn start_input_worker(
    allowed: Arc<AtomicBool>,
    monitor: Arc<AtomicUsize>,
    messages: Sender<String>,
) -> std::sync::mpsc::SyncSender<Control> {
    use std::sync::mpsc::{self, RecvTimeoutError};
    use enigo::{Button, Direction, Enigo, Mouse, Settings};
    let (tx, rx) = mpsc::sync_channel::<Control>(128);
    std::thread::spawn(move || {
        let Ok(mut enigo) = Enigo::new(&Settings::default()) else {
            let _ = messages.send("远程输入初始化失败；请检查 Windows 桌面会话权限".into());
            return;
        };
        let mut held = false;
        let mut last_action = std::time::Instant::now();
        loop {
            if !allowed.load(Ordering::SeqCst) && held {
                let _ = enigo.button(Button::Left, Direction::Release);
                held = false;
            }
            let command = match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => {
                    if held && last_action.elapsed() > Duration::from_secs(20) {
                        let _ = enigo.button(Button::Left, Direction::Release);
                        held = false;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if !allowed.load(Ordering::SeqCst) { continue; }
            last_action = std::time::Instant::now();
            match command.kind.as_str() {
                "down" if valid_pointer(&command) => {
                    if !held {
                        handle_control(&mut enigo, &command, monitor.load(Ordering::SeqCst));
                        held = true;
                    }
                }
                "up" => {
                    // Release even when pointer position is outside the current
                    // monitor after a drag, display switch or zoom gesture.
                    if held {
                        let _ = enigo.button(Button::Left, Direction::Release);
                        held = false;
                    }
                }
                "right_click" | "move" if valid_pointer(&command) =>
                    handle_control(&mut enigo, &command, monitor.load(Ordering::SeqCst)),
                "text" | "key" | "scroll" =>
                    handle_control(&mut enigo, &command, monitor.load(Ordering::SeqCst)),
                _ => {}
            }
        }
        if held { let _ = enigo.button(Button::Left, Direction::Release); }
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
fn handle_control(enigo: &mut enigo::Enigo, command: &Control, index: usize) {
    use enigo::{Axis, Button, Coordinate, Direction, Keyboard, Mouse};
    if command.kind == "text" {
        if let Some(value) = &command.text {
            if value.chars().count() <= 250 { let _ = enigo.text(value); }
        }
        return;
    }
    if command.kind == "key" {
        use enigo::Key;
        let Some(name) = command.key.as_deref() else { return; };
        if !crate::remote_ops::valid_remote_key(name) { return; }
        // Whitelisted combinations are emitted with a guaranteed Control
        // release, even if injecting the letter fails.
        if let Some(letter) = match name {
            "ctrl_a" => Some('a'), "ctrl_c" => Some('c'),
            "ctrl_v" => Some('v'), "ctrl_x" => Some('x'),
            "ctrl_z" => Some('z'), "ctrl_y" => Some('y'),
            _ => None,
        } {
            if enigo.key(Key::Control, Direction::Press).is_ok() {
                let _ = enigo.key(Key::Unicode(letter), Direction::Click);
                let _ = enigo.key(Key::Control, Direction::Release);
            }
            return;
        }
        let key = match name {
            "enter" => Key::Return,
            "backspace" => Key::Backspace,
            "tab" => Key::Tab,
            "escape" => Key::Escape,
            "delete" => Key::Delete,
            "up" => Key::UpArrow,
            "down" => Key::DownArrow,
            "left" => Key::LeftArrow,
            "right" => Key::RightArrow,
            "home" => Key::Home,
            "end" => Key::End,
            "page_up" => Key::PageUp,
            "page_down" => Key::PageDown,
            "space" => Key::Space,
            _ => return,
        };
        let _ = enigo.key(key, Direction::Click);
        return;
    }
    if command.kind == "scroll" {
        if let Some(delta) = command.delta {
            if (-10..=10).contains(&delta) && delta != 0 {
                let _ = enigo.scroll(delta, Axis::Vertical);
            }
        }
        return;
    }
    if !matches!(command.kind.as_str(), "down" | "up" | "move" | "right_click") { return; }
    let (Some(x), Some(y)) = (command.x, command.y) else { return; };
    if !x.is_finite() || !y.is_finite()
        || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) { return; }
    if let Ok(screens) = screenshots::Screen::all() {
        if let Some(screen) = screens.get(index) {
            let info = &screen.display_info;
            let px = info.x + (x * (info.width.saturating_sub(1) as f64)) as i32;
            let py = info.y + (y * (info.height.saturating_sub(1) as f64)) as i32;
            let _ = enigo.move_mouse(px, py, Coordinate::Abs);
            match command.kind.as_str() {
                "down" => { let _ = enigo.button(Button::Left, Direction::Press); }
                "up" => { let _ = enigo.button(Button::Left, Direction::Release); }
                "right_click" => { let _ = enigo.button(Button::Right, Direction::Click); }
                _ => {}
            }
        }
    }
}

use crate::protocol;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, Arc, mpsc::Sender}, time::Duration};
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
    pub messages: Sender<String>,
}

pub fn start(config: HostConfig) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(error) => {
                let _ = config.messages.send(format!("运行时初始化失败: {error}"));
                config.running.store(false, Ordering::SeqCst);
                return;
            }
        };
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
                        let _ = config.messages.send("中转访问口令错误（HTTP 401/403），已停止自动重试".into());
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
    let (mut outgoing, mut incoming) = socket.split();
    let mut ticker = tokio::time::interval(Duration::from_millis(150));
    let mut metadata_tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !config.running.load(Ordering::SeqCst) { break; }
                #[cfg(windows)]
                {
                    if let Ok(jpg) = capture_jpeg(config.monitor_index.load(Ordering::SeqCst)) {
                        let packet = protocol::encrypt(&config.secret, protocol::FRAME, &jpg)?;
                        tokio::time::timeout(Duration::from_secs(8), outgoing.send(Message::Binary(packet)))
                            .await.context("video send timed out")??;
                    }
                }
            }
            _ = metadata_tick.tick() => {
                #[cfg(windows)]
                {
                    let count = screenshots::Screen::all().map(|screens| screens.len()).unwrap_or(0);
                    let selected = config.monitor_index.load(Ordering::SeqCst).min(count.saturating_sub(1));
                    let info = serde_json::json!({"count": count.min(16), "selected": selected}).to_string();
                    let packet = protocol::encrypt(&config.secret, protocol::MONITORS, info.as_bytes())?;
                    outgoing.send(Message::Binary(packet)).await?;
                }
            }
            next = incoming.next() => {
                match next {
                    Some(Ok(Message::Binary(packet))) => {
                        if let Ok((protocol::CONTROL, bytes)) = protocol::decrypt(&config.secret, &packet) {
                            #[cfg(windows)]
                            {
                                if let Ok(command) = serde_json::from_slice::<Control>(&bytes) {
                                    match command.kind.as_str() {
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
                                            handle_control(&command, config.monitor_index.load(Ordering::SeqCst));
                                        }
                                        _ => {}
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
    let _ = outgoing.close().await;
    let _ = config.messages.send("连接已断开".into());
    Ok(())
}

#[cfg(windows)]
fn capture_jpeg(index: usize) -> Result<Vec<u8>> {
    use image::{codecs::jpeg::JpegEncoder, DynamicImage, RgbaImage, imageops::FilterType};
    let screen = screenshots::Screen::all()?.into_iter().nth(index).context("selected display missing")?;
    let capture = screen.capture()?;
    let (w, h) = (capture.width(), capture.height());
    let rgba = RgbaImage::from_raw(w, h, capture.into_raw()).context("bad screenshot buffer")?;
    let img = DynamicImage::ImageRgba8(rgba).resize(1280, 800, FilterType::Triangle);
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 58).encode_image(&img)?;
    Ok(output)
}

#[derive(Deserialize)]
struct Control {
    #[serde(rename="type")] kind: String,
    x: Option<f64>,
    y: Option<f64>,
    text: Option<String>,
    index: Option<usize>,
    delta: Option<i32>,
}

#[cfg(windows)]
fn handle_control(command: &Control, index: usize) {
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
    let Ok(mut enigo) = Enigo::new(&Settings::default()) else { return; };
    if command.kind == "text" {
        if let Some(value) = &command.text {
            if value.chars().count() <= 250 { let _ = enigo.text(value); }
        }
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

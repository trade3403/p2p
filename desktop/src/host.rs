use crate::protocol;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc, mpsc::Sender}, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, http::HeaderValue, Message}};

#[derive(Clone)]
pub struct HostConfig {
    pub server: String,
    pub relay_token: String,
    pub secret: [u8; 32],
    pub running: Arc<AtomicBool>,
    pub allow_input: Arc<AtomicBool>,
    pub messages: Sender<String>,
}

pub fn start(config: HostConfig) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(error) => { let _ = config.messages.send(format!("运行时错误: {error}")); return; }
        };
        if let Err(error) = runtime.block_on(run(&config)) {
            let _ = config.messages.send(format!("连接失败: {error:#}"));
        }
        config.running.store(false, Ordering::SeqCst);
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
    let (socket, _) = connect_async(request).await.context("relay not reachable")?;
    let _ = config.messages.send("已连接中转服务器 · 远控可用".into());
    let (mut outgoing, mut incoming) = socket.split();
    let mut ticker = tokio::time::interval(Duration::from_millis(150));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !config.running.load(Ordering::SeqCst) { break; }
                #[cfg(windows)]
                {
                    if let Ok(jpg) = capture_jpeg() {
                        let packet = protocol::encrypt(&config.secret, protocol::FRAME, &jpg)?;
                        outgoing.send(Message::Binary(packet)).await?;
                    }
                }
            }
            next = incoming.next() => {
                match next {
                    Some(Ok(Message::Binary(packet))) => {
                        if config.allow_input.load(Ordering::SeqCst) {
                            if let Ok((protocol::CONTROL, bytes)) = protocol::decrypt(&config.secret, &packet) {
                                #[cfg(windows)] handle_control(&bytes);
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
fn capture_jpeg() -> Result<Vec<u8>> {
    use image::{codecs::jpeg::JpegEncoder, DynamicImage, RgbaImage, imageops::FilterType};
    let screen = screenshots::Screen::all()?.into_iter().next().context("no display")?;
    let capture = screen.capture()?;
    let (w, h) = (capture.width(), capture.height());
    let rgba = RgbaImage::from_raw(w, h, capture.into_raw()).context("bad screenshot buffer")?;
    let img = DynamicImage::ImageRgba8(rgba).resize(1280, 800, FilterType::Triangle);
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 58).encode_image(&img)?;
    Ok(output)
}

#[derive(Deserialize)]
struct Control { #[serde(rename="type")] kind: String, x: Option<f64>, y: Option<f64>, text: Option<String> }

#[cfg(windows)]
fn handle_control(data: &[u8]) {
    use enigo::{Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
    if data.len() > 2048 { return; }
    let Ok(command) = serde_json::from_slice::<Control>(data) else { return; };
    let Ok(mut enigo) = Enigo::new(&Settings::default()) else { return; };
    if command.kind == "text" {
        if let Some(text) = command.text { if text.chars().count() <= 250 { let _ = enigo.text(&text); } }
        return;
    }
    let (Some(x), Some(y)) = (command.x, command.y) else { return; };
    if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) { return; }
    if let Ok(screens) = screenshots::Screen::all() {
        if let Some(screen) = screens.first() {
            let info = &screen.display_info;
            let px = info.x + (x * (info.width.saturating_sub(1) as f64)) as i32;
            let py = info.y + (y * (info.height.saturating_sub(1) as f64)) as i32;
            let _ = enigo.move_mouse(px, py, Coordinate::Abs);
            match command.kind.as_str() {
                "down" => { let _ = enigo.button(Button::Left, Direction::Press); }
                "up" => { let _ = enigo.button(Button::Left, Direction::Release); }
                "move" => {}
                _ => {}
            }
        }
    }
}

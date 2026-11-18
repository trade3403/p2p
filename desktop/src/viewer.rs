//! Windows viewer for the existing authenticated VPS relay protocol.
//! Only operates on a device whose owner has explicitly enabled remote access.
use crate::protocol;
use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc, mpsc::{Receiver, Sender}}, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, http::HeaderValue, Message}};

pub enum ViewerEvent {
    State(String),
    Connected,
    Disconnected,
    Jpeg(Vec<u8>,u32,u32),
    Monitors(String),
    Clipboard(String),
}

pub struct ViewerConfig {
    pub server: String,
    pub token: String,
    pub secret: [u8; 32],
    pub active: Arc<AtomicBool>,
    pub events: Sender<ViewerEvent>,
    pub controls: Receiver<String>,
}

fn endpoint(server: &str, secret: &[u8;32]) -> Result<String> {
    let mut url = url::Url::parse(server.trim()).context("Invalid relay URL")?;
    match url.scheme() {
        "https" => { url.set_scheme("wss").map_err(|_| anyhow!("Invalid URL scheme"))?; },
        "wss" => {},
        _ => return Err(anyhow!("Remote viewer requires TLS/WSS")),
    }
    if url.username() != "" || url.password().is_some() || url.host_str().is_none() {
        return Err(anyhow!("Credentials are not allowed in relay URL"));
    }
    url.set_path("/relay");
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("room", &protocol::room_id(secret))
        .append_pair("role", "viewer");
    Ok(url.to_string())
}

pub fn start(config: ViewerConfig) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = config.events.send(ViewerEvent::State(format!("Viewer 运行时错误：{err}")));
                config.active.store(false, Ordering::SeqCst);
                return;
            }
        };
        if let Err(err) = runtime.block_on(run(&config)) {
            let _ = config.events.send(ViewerEvent::State(format!("连接失败：{err:#}")));
        }
        config.active.store(false, Ordering::SeqCst);
        let _ = config.events.send(ViewerEvent::Disconnected);
    });
}

async fn run(config: &ViewerConfig) -> Result<()> {
    if config.token.trim().len() < 32 {
        return Err(anyhow!("Relay Token 必须至少 32 位"));
    }
    let url = endpoint(&config.server, &config.secret)?;
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", config.token.trim()))?,
    );
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(12), connect_async(request))
        .await.context("WSS connection timeout")??;
    let _ = config.events.send(ViewerEvent::Connected);
    // Windows Viewer is JPEG-only. Send an AES-GCM-authenticated codec request
    // as soon as the viewer joins. Host defaults are left unchanged.
    let codec_request = protocol::encrypt(
        &config.secret, protocol::CONTROL,
        br#"{"type":"video_codec","codec":"jpeg"}"#,
    )?;
    socket.send(Message::Binary(codec_request)).await?;
    let _ = config.events.send(ViewerEvent::State("已请求远端切换 JPEG 兼容画面…".into()));
    while config.active.load(Ordering::SeqCst) {
        tokio::select! {
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Binary(packet))) => {
                        if let Ok((kind, payload)) = protocol::decrypt(&config.secret, &packet) {
                            match kind {
                                protocol::FRAME if payload.len() <= 3*1024*1024 => {
                                    // Do not interpret arbitrary ciphertext as remote UI.
                                    if payload.starts_with(&[0xff,0xd8,0xff]) {
                                        if let Ok(img) = ::image::load_from_memory_with_format(&payload, ::image::ImageFormat::Jpeg) {
                                            if img.width() > 0 && img.height() > 0
                                                && img.width() <= 8192 && img.height() <= 8192 {
                                                let _ = config.events.send(ViewerEvent::Jpeg(payload,img.width(),img.height()));
                                            }
                                        }
                                    }
                                },
                                protocol::MONITORS => {
                                    let _ = config.events.send(ViewerEvent::Monitors(String::from_utf8_lossy(&payload).into_owned()));
                                },
                                protocol::CLIPBOARD => {
                                    let _ = config.events.send(ViewerEvent::Clipboard(String::from_utf8_lossy(&payload).into_owned()));
                                },
                                _ => {}
                            }
                        }
                    },
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(err)) => return Err(err.into()),
                    _ => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(30)) => {
                while let Ok(command) = config.controls.try_recv() {
                    if command.len() > 8192 { continue; }
                    let encrypted = protocol::encrypt(&config.secret, protocol::CONTROL, command.as_bytes())?;
                    socket.send(Message::Binary(encrypted)).await?;
                }
            }
        }
    }
    let _ = socket.close(None).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_role_viewer_with_existing_room_scheme() {
        let key = [3u8;32];
        let url = endpoint("wss://p2p.aikissgo.com", &key).unwrap();
        assert!(url.contains("role=viewer"));
        assert!(url.contains(&protocol::room_id(&key)));
        assert!(endpoint("ws://public.example.com", &key).is_err());
    }
}

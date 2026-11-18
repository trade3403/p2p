#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app_logging;
#[cfg(windows)]
mod tray;
mod host;
#[cfg(windows)]
mod host_pairing;
mod protocol;
#[allow(dead_code)]
mod pairing_crypto;
#[allow(dead_code)]
mod pairing_api;
mod reconnect;
mod local_settings;
mod remote_ops;
mod trusted_profile;
mod video;
mod lan;
mod stun;
mod udp_punch;
mod udp_video;
mod viewer;
mod viewer_geometry;
mod tls;

use iced::{Font, Task, window, clipboard, widget::{button, checkbox, column, container, horizontal_space, vertical_space, row, scrollable, text, text_input, image, mouse_area}, Border, Color, Element, Length, Subscription, Theme};
use std::{sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc, mpsc::{self, Receiver, Sender}}, time::{Duration, Instant}};

fn main() -> iced::Result {
    app_logging::init();
    std::panic::set_hook(Box::new(|panic| {
        tracing::error!(panic = %panic, "fatal application panic");
    }));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "P2P Remote starting");
    // Must run before App::default or any Host/Viewer WSS thread starts.
    tls::initialize();
    iced::application("P2P Remote · 远程桌面", update, view)
        .theme(|_| Theme::TokyoNight)
        .exit_on_close_request(false)
        .default_font(Font::with_name("Microsoft YaHei UI"))
        .subscription(|_| Subscription::batch([
            iced::time::every(Duration::from_millis(250)).map(|_| Msg::Tick),
            window::resize_events().map(|(_, size)| Msg::WindowResized(size.width, size.height)),
            window::close_requests().map(Msg::CloseRequested),
        ]))
        .window_size((1180.0, 780.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page { Home, Remote, Settings, Network }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerScale { Compact, Standard, Large }

impl ViewerScale {
    fn dimensions(self) -> (f32, f32) {
        match self {
            Self::Compact => (560.0, 315.0),
            Self::Standard => (700.0, 394.0),
            Self::Large => (840.0, 473.0),
        }
    }
}

#[derive(Debug, Clone)]
struct PairingRequestPrompt {
    request_id: String,
    viewer_fingerprint: String,
}

struct App {
    page: Page,
    viewer_scale: ViewerScale,
    viewer_fullscreen: bool,
    window_size: (f32, f32),
    show_session_key: bool,
    server: String,
    relay_token: String,
    device_number: String,
    pairing_code: String,
    pairing_expires_at: i64,
    pending_pairing_request: Option<PairingRequestPrompt>,
    #[cfg(windows)]
    pairing_decision_tx: Option<Sender<host_pairing::PairingDecision>>,
    secret: [u8;32],
    running: Arc<AtomicBool>,
    allow_input: Arc<AtomicBool>,
    allow_clipboard: Arc<AtomicBool>,
    launch_on_login: bool,
    unattended_input: bool,
    trust_saved: bool,
    monitor_index: Arc<AtomicUsize>,
    lan_direct: Arc<AtomicBool>,
    lan_active: Arc<AtomicBool>,
    allow_udp_probe: Arc<AtomicBool>,
    allow_udp_preview: Arc<AtomicBool>,
    udp_active: Arc<AtomicBool>,
    h264_enabled: Arc<AtomicBool>,
    target_fps: Arc<AtomicU32>,
    sent_frames: Arc<AtomicU64>,
    dropped_frames: Arc<AtomicU64>,
    status: String,
    diagnostics: String,
    viewer_key: String,
    viewer_device_number: String,
    viewer_pairing_code: String,
    viewer_pairing_active: Arc<AtomicBool>,
    viewer_pairing_cancel: Arc<AtomicBool>,
    show_legacy_viewer_key: bool,
    viewer_text: String,
    viewer_status: String,
    viewer_monitors: String,
    viewer_clipboard: String,
    viewer_frame: Option<image::Handle>,
    viewer_online: bool,
    viewer_wait_since: Option<Instant>,
    viewer_pointer: (f32, f32),
    viewer_frame_size: (u32,u32),
    viewer_pointer_valid: bool,
    viewer_monitor_info: Option<viewer_geometry::Monitors>,
    viewer_active: Arc<AtomicBool>,
    viewer_tx: Sender<String>,
    viewer_rx: Receiver<viewer::ViewerEvent>,
    tx: Sender<String>,
    rx: Receiver<String>,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    #[cfg(windows)]
    tray_initialized: bool,
    tray_input_paused: bool,
    hide_on_startup: bool,
}
impl Default for App {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let (viewer_tx, _unused_receiver) = mpsc::channel();
        let (_viewer_event_tx, viewer_rx) = mpsc::channel();
        let autostart = std::env::args().any(|arg| arg == "--autostart");
        // Failed DPAPI decryption never silently recreates a trusted session.
        let trusted = trusted_profile::load().ok().flatten();
        let secret = trusted.as_ref()
            .and_then(|p| hex::decode(&p.session_hex).ok())
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .unwrap_or_else(protocol::new_secret);
        let mut app = Self {
            page: Page::Home,
            viewer_scale: ViewerScale::Standard,
            viewer_fullscreen: false,
            window_size: (1180.0, 780.0),
            show_session_key: false,
            server: trusted.as_ref().map(|p| p.relay_url.clone())
                .unwrap_or_else(local_settings::load_relay_url),
            relay_token: trusted.as_ref().map(|p| p.relay_token.clone()).unwrap_or_default(),
            device_number: String::new(),
            pairing_code: String::new(),
            pairing_expires_at: 0,
            pending_pairing_request: None,
            #[cfg(windows)]
            pairing_decision_tx: None,
            secret,
            running: Arc::new(AtomicBool::new(false)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(false)),
            lan_active: Arc::new(AtomicBool::new(false)),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(12)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            launch_on_login: trusted.as_ref().map(|p| p.launch_on_login).unwrap_or(false),
            unattended_input: trusted.as_ref().map(|p| p.unattended_input).unwrap_or(false),
            trust_saved: trusted.is_some(),
            status: "未连接 · 手动启动或启用登录后自动连接".into(),
            diagnostics: "尚未探测 STUN 公网 UDP 地址".into(),
            viewer_key: String::new(),
            viewer_device_number: String::new(),
            viewer_pairing_code: String::new(),
            viewer_pairing_active: Arc::new(AtomicBool::new(false)),
            viewer_pairing_cancel: Arc::new(AtomicBool::new(false)),
            show_legacy_viewer_key: false,
            viewer_text: String::new(),
            viewer_status: "未连接远程 Windows".into(),
            viewer_monitors: String::new(),
            viewer_clipboard: String::new(),
            viewer_frame: None,
            viewer_online: false,
            viewer_wait_since: None,
            viewer_pointer: (0.5, 0.5),
            viewer_frame_size: (0,0),
            viewer_pointer_valid: false,
            viewer_monitor_info: None,
            viewer_active: Arc::new(AtomicBool::new(false)),
            viewer_tx,
            viewer_rx,
            tx,
            rx,
            #[cfg(windows)]
            tray: None,
            #[cfg(windows)]
            tray_initialized: false,
            tray_input_paused: false,
            hide_on_startup: autostart && trusted.as_ref().map(|p| p.launch_on_login).unwrap_or(false),
        };
        // Auto-start is opt-in and only runs after normal user logon.
        // It is never a Windows service and never attempts secure-desktop capture.
        if autostart && app.trust_saved && app.launch_on_login
        {
            app.allow_input.store(app.unattended_input, Ordering::SeqCst);
            app.running.store(true, Ordering::SeqCst);
            app.status = "已按此前授权登录后启动 · 正在连接 VPS".into();
            host::start(host::HostConfig {
                server: app.server.clone(),
                relay_token: app.relay_token.clone(),
                secret: app.secret,
                running: app.running.clone(),
                allow_input: app.allow_input.clone(),
                allow_clipboard: app.allow_clipboard.clone(),
                monitor_index: app.monitor_index.clone(),
                lan_direct: app.lan_direct.clone(),
                lan_active: app.lan_active.clone(),
                allow_udp_probe: app.allow_udp_probe.clone(),
                allow_udp_preview: app.allow_udp_preview.clone(),
                udp_active: app.udp_active.clone(),
                h264_enabled: app.h264_enabled.clone(),
                target_fps: app.target_fps.clone(),
                sent_frames: app.sent_frames.clone(),
                dropped_frames: app.dropped_frames.clone(),
                messages: app.tx.clone(),
            });
        }
        app
    }
}
// Final application shutdown must stop every session. Hiding the window is
// handled separately and deliberately does not drop this application state.
impl Drop for App {
    fn drop(&mut self) {
        tracing::info!("P2P Remote shutting down; stopping host and viewer sessions");
        self.running.store(false, Ordering::SeqCst);
        self.viewer_active.store(false, Ordering::SeqCst);
        self.allow_input.store(false, Ordering::SeqCst);
        self.allow_clipboard.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
fn submit_pairing_decision(state: &mut App, approved: bool) {
    #[cfg(windows)]
    {
        if let Some(request) = state.pending_pairing_request.take() {
            if let Some(sender) = state.pairing_decision_tx.as_ref() {
                let _ = sender.send(host_pairing::PairingDecision::Respond {
                    request_id: request.request_id,
                    approved,
                });
                state.status = if approved { "已提交允许连接决定".into() } else { "已提交拒绝连接决定".into() };
            } else {
                state.status = "数字配对服务已停止，请重新启动共享".into();
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = approved;
        state.status = "数字配对仅在 Windows Host 上提供".into();
    }
}

enum Msg { Navigate(Page), ViewerZoom(ViewerScale), ToggleViewerFullscreen,
    ViewerWindow(Option<window::Id>), WindowResized(f32, f32),
    CloseRequested(window::Id), ShowWindow(Option<window::Id>), HideWindow(Option<window::Id>), ExitWindow(Option<window::Id>),
    ToggleSessionKey, CopyDeviceNumber, CopyPairingKey, CopyPairingCode, ApprovePairing, DenyPairing, Server(String), Token(String), Allow(bool), Clipboard(bool), NextMonitor,
    LoginStart(bool), UnattendedInput(bool), SaveTrust, RevokeTrust,
    ViewerPairingNumber(String), ViewerPairingCode(String), ViewerPair, ToggleLegacyViewerKey,
    H264(bool), Fps(u32), Lan(bool), AllowUdpProbe(bool), AllowUdpPreview(bool), ProbeNetwork, Start, Stop, NewKey, Tick, ViewerKey(String), ViewerText(String), ViewerConnect, ViewerDisconnect, ViewerMove(iced::Point), ViewerDown, ViewerUp, ViewerRight, ViewerSendText, ViewerScroll(i32), ViewerMonitor(usize), ViewerClipboardGet, ViewerClipboardSet, ViewerKeyPress(&'static str) }

fn update(state: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::CloseRequested(id) => {
            #[cfg(windows)]
            {
                if state.tray.is_some() {
                    state.status = "窗口已隐藏到系统托盘；已授权的共享会话继续运行".into();
                    tracing::info!("main window hidden to system tray");
                    return window::change_mode(id, window::Mode::Hidden);
                }
                state.status = "系统托盘未能初始化，为避免失去入口，窗口保持打开".into();
                tracing::error!("close request ignored because system tray is unavailable");
                return Task::none();
            }
            #[cfg(not(windows))]
            {
                return window::close(id);
            }
        }
        Msg::ShowWindow(Some(id)) => {
            state.hide_on_startup = false;
            return Task::batch([
                window::change_mode(id, window::Mode::Windowed),
                window::gain_focus(id),
            ]);
        }
        Msg::ShowWindow(None) => return Task::none(),
        Msg::HideWindow(Some(id)) => {
            state.status = "已登录启动 · P2P Remote 在系统托盘运行".into();
            return window::change_mode(id, window::Mode::Hidden);
        }
        Msg::HideWindow(None) => return Task::none(),
        Msg::ExitWindow(Some(id)) => return window::close(id),
        Msg::ExitWindow(None) => return Task::none(),
        Msg::ToggleViewerFullscreen if state.page == Page::Remote => {
            return window::get_latest().map(Msg::ViewerWindow);
        }
        Msg::ViewerWindow(Some(id)) if state.page == Page::Remote => {
            let next = !state.viewer_fullscreen;
            state.viewer_fullscreen = next;
            state.viewer_pointer_valid = false;
            let mode = if next { window::Mode::Fullscreen } else { window::Mode::Windowed };
            return window::change_mode(id, mode);
        }
        Msg::WindowResized(width, height) if width.is_finite() && height.is_finite()
            && width >= 320.0 && height >= 240.0 =>
        {
            state.window_size = (width, height);
        }
        Msg::Navigate(page) => {
            // Never leave a remote button held while its viewer canvas disappears.
            if state.page == Page::Remote && page != Page::Remote && state.viewer_online {
                let _ = state.viewer_tx.send(serde_json::json!({
                    "type":"up", "x":state.viewer_pointer.0, "y":state.viewer_pointer.1
                }).to_string());
                state.viewer_pointer_valid = false;
            }
            state.page = page;
        }
        Msg::ViewerZoom(scale) => {
            if state.viewer_online && state.viewer_pointer_valid {
                let _ = state.viewer_tx.send(serde_json::json!({
                    "type":"up", "x":state.viewer_pointer.0, "y":state.viewer_pointer.1
                }).to_string());
            }
            state.viewer_scale = scale;
            state.viewer_pointer_valid = false;
        }
        Msg::ToggleSessionKey => state.show_session_key = !state.show_session_key,
        Msg::CopyDeviceNumber => {
            if state.device_number.is_empty() {
                state.status = "请先启动本机共享，等待服务器分配设备号".into();
                return Task::none();
            }
            state.status = "已复制服务器分配的 9 位设备号".into();
            return clipboard::write(state.device_number.clone());
        }
        Msg::CopyPairingCode => {
            if state.pairing_code.is_empty() {
                state.status = "动态验证码尚未生成".into();
                return Task::none();
            }
            state.status = "已复制动态验证码".into();
            return clipboard::write(state.pairing_code.clone());
        }
        Msg::ApprovePairing => submit_pairing_decision(state, true),
        Msg::DenyPairing => submit_pairing_decision(state, false),
        Msg::CopyPairingKey => {
            state.status = "已复制配对密钥；请仅通过可信渠道分享".into();
            return clipboard::write(hex::encode(state.secret));
        }
        Msg::ViewerKey(value) if !state.viewer_active.load(Ordering::SeqCst) => {
            state.viewer_key = value.trim().to_owned();
        }
        Msg::ViewerText(value) => state.viewer_text = value.chars().take(250).collect(),
        Msg::ViewerConnect if !state.viewer_active.load(Ordering::SeqCst) => {
            let Ok(raw) = hex::decode(state.viewer_key.trim()) else {
                state.viewer_status = "远程 Windows 密钥必须为 64 位十六进制".into();
                return Task::none();
            };
            let Ok(secret) = <[u8;32]>::try_from(raw) else {
                state.viewer_status = "远程 Windows 密钥必须是 32 字节".into();
                return Task::none();
            };
            // Fresh control queue and independent cancellation token on EVERY
            // connect. Old sessions cannot accidentally consume new commands.
            let (control_tx, controls) = mpsc::channel();
            state.viewer_tx = control_tx;
            let (events, rx) = mpsc::channel();
            state.viewer_rx = rx;
            state.viewer_active = Arc::new(AtomicBool::new(true));
            state.viewer_status = "正在连接远程 Windows...".into();
            state.viewer_online = false;
            state.viewer_wait_since = Some(Instant::now());
            state.viewer_frame = None;
            state.viewer_monitor_info = None;
            state.viewer_frame_size = (0,0);
            state.viewer_pointer_valid = false;
            viewer::start(viewer::ViewerConfig {
                server: state.server.clone(),
                token: state.relay_token.clone(),
                secret,
                active: state.viewer_active.clone(),
                events,
                controls,
            });
        }
        Msg::ViewerDisconnect => {
            if state.viewer_online {
                let _ = state.viewer_tx.send(serde_json::json!({
                    "type":"up", "x":state.viewer_pointer.0, "y":state.viewer_pointer.1
                }).to_string());
            }
            state.viewer_active.store(false, Ordering::SeqCst);
            state.viewer_online = false;
            state.viewer_wait_since = None;
            state.viewer_pointer_valid = false;
            state.viewer_frame = None;
            state.viewer_status = "手动断开连接".into();
        }
        Msg::ViewerMove(position) if state.viewer_online => {
            if let Some((x,y)) = viewer_geometry::normalized_pointer(
                position.x, position.y,
                viewer_geometry::viewer_dimensions(state.viewer_fullscreen,
                    state.window_size, state.viewer_scale.dimensions()).0,
                viewer_geometry::viewer_dimensions(state.viewer_fullscreen,
                    state.window_size, state.viewer_scale.dimensions()).1,
                state.viewer_frame_size.0, state.viewer_frame_size.1
            ) {
                state.viewer_pointer = (x,y);
                state.viewer_pointer_valid = true;
                let _ = state.viewer_tx.send(serde_json::json!({"type":"move","x":x,"y":y}).to_string());
            } else {
                state.viewer_pointer_valid = false;
            }
        }
        Msg::ViewerDown if state.viewer_online && state.viewer_pointer_valid => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"down","x":state.viewer_pointer.0,"y":state.viewer_pointer.1}).to_string());
        }
        Msg::ViewerUp if state.viewer_online => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"up","x":state.viewer_pointer.0,"y":state.viewer_pointer.1}).to_string());
        }
        Msg::ViewerRight if state.viewer_online && state.viewer_pointer_valid => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"right_click","x":state.viewer_pointer.0,"y":state.viewer_pointer.1}).to_string());
        }
        Msg::ViewerScroll(delta) if state.viewer_online => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"scroll","delta":delta}).to_string());
        }
        Msg::ViewerSendText if state.viewer_online && !state.viewer_text.is_empty() => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"text","text":state.viewer_text}).to_string());
            state.viewer_text.clear();
        }
        Msg::ViewerMonitor(index) if state.viewer_online => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"monitor","index":index}).to_string());
        }
        Msg::ViewerClipboardGet if state.viewer_online => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"clipboard_get"}).to_string());
        }
        Msg::ViewerClipboardSet if state.viewer_online && !state.viewer_text.is_empty() => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"clipboard_set","text":state.viewer_text}).to_string());
            state.viewer_status = "剪贴板文字已请求发送（需被控端授权）".into();
        }
        Msg::ViewerKeyPress(key) if state.viewer_online => {
            let _ = state.viewer_tx.send(serde_json::json!({"type":"key","key":key}).to_string());
        }
        Msg::Server(s) if !state.running.load(Ordering::SeqCst) => state.server = s,
        Msg::Token(s) if !state.running.load(Ordering::SeqCst) => state.relay_token = s,
        Msg::Allow(value) => {
            state.allow_input.store(value, Ordering::SeqCst);
            state.tray_input_paused = false;
        },
        Msg::Clipboard(value) => state.allow_clipboard.store(value, Ordering::SeqCst),
        Msg::ProbeNetwork => {
            state.diagnostics = "正在发送 STUN UDP 探测（最多约 2 秒）".into();
            let relay = state.server.clone();
            let tx = state.tx.clone();
            std::thread::spawn(move || {
                let result = match stun::probe_from_relay(&relay) {
                    Ok((mapped, rtt)) => format!(
                        "[STUN] 探测成功：出口 {} · UDP 往返 {} ms。只是一次性 NAT 映射，不代表可直接穿透。",
                        mapped, rtt.as_millis()
                    ),
                    Err(error) => format!(
                        "[STUN] 探测失败：{}。请确认 VPS 已启用 UDP 3478 并检查防火墙。",
                        error
                    ),
                };
                let _ = tx.send(result);
            });
        }
        Msg::AllowUdpPreview(value) if !state.running.load(Ordering::SeqCst) => {
            state.allow_udp_preview.store(value, Ordering::SeqCst);
        },
        Msg::AllowUdpProbe(value) if !state.running.load(Ordering::SeqCst) => {
            state.allow_udp_probe.store(value, Ordering::SeqCst);
        },
        Msg::Lan(value) if !state.running.load(Ordering::SeqCst) => {
            state.lan_direct.store(value, Ordering::SeqCst);
        },
        Msg::H264(value) => state.h264_enabled.store(value, Ordering::SeqCst),
        Msg::Fps(value) if matches!(value, 6|12|20) => state.target_fps.store(value, Ordering::SeqCst),
        Msg::LoginStart(value) => state.launch_on_login = value,
        Msg::UnattendedInput(value) => state.unattended_input = value,
        Msg::SaveTrust => {
            let profile = trusted_profile::TrustedProfile {
                relay_url: state.server.trim().to_owned(),
                relay_token: state.relay_token.trim().to_owned(),
                session_hex: hex::encode(state.secret),
                launch_on_login: state.launch_on_login,
                unattended_input: state.unattended_input,
            };
            match trusted_profile::save(&profile) {
                Ok(()) => {
                    match trusted_profile::set_login_start(state.launch_on_login) {
                        Ok(()) => {
                            state.trust_saved = true;
                            state.status = if state.launch_on_login {
                                "已加密保存凭据并授权 Windows 用户登录后自动启动".into()
                            } else {
                                "已加密保存可信配对；登录后自动启动尚未开启".into()
                            };
                        }
                        Err(err) => {
                            // Fail closed: do not claim auto-start worked.
                            state.launch_on_login = false;
                            let _ = trusted_profile::revoke();
                            state.trust_saved = false;
                            state.status = format!("无法修改登录自启设置，配对未保存：{err}");
                        }
                    }
                }
                Err(err) => state.status = format!("无法安全保存配对：{err}"),
            }
        }
        Msg::RevokeTrust => {
            state.running.store(false, Ordering::SeqCst);
            state.allow_input.store(false, Ordering::SeqCst);
            state.allow_clipboard.store(false, Ordering::SeqCst);
            state.relay_token.clear();
            state.secret = protocol::new_secret();
            state.launch_on_login = false;
            state.unattended_input = false;
            let registry = trusted_profile::set_login_start(false);
            let vault = trusted_profile::revoke();
            state.trust_saved = vault.is_err();
            state.status = if registry.is_ok() && vault.is_ok() {
                "已撤销可信配对并轮换密钥；旧手机将无法重新加入新会话".into()
            } else {
                "撤销时出现本地系统错误，请检查 Windows 用户启动项与凭据文件".into()
            };
        }
        Msg::NextMonitor => {
            #[cfg(windows)]
            {
                let count = screenshots::Screen::all().map(|screens| screens.len()).unwrap_or(1).max(1);
                let current = state.monitor_index.load(Ordering::SeqCst);
                state.monitor_index.store((current + 1) % count, Ordering::SeqCst);
            }
        },
        Msg::Start if !state.running.load(Ordering::SeqCst) => {
            tracing::info!("starting local host numeric pairing service");
            if state.relay_token.trim().is_empty() {
                state.status = "请填写私测中转访问口令".into();
                return Task::none();
            }
            if let Err(err) = local_settings::save_relay_url(state.server.trim()) {
                state.status = format!("服务器地址无效或无法保存：{err}");
                return Task::none();
            }
            let (device_id_hex, signing_seed) = match trusted_profile::load_or_create_device_identity() {
                Ok(value) => value,
                Err(error) => {
                    state.status = format!("本机设备身份无法安全保存，已停止启动：{error}");
                    return Task::none();
                }
            };
            let device_id = match hex::decode(device_id_hex) {
                Ok(value) => match <[u8; 16]>::try_from(value) {
                    Ok(value) => value,
                    Err(_) => {
                        state.status = "本机设备身份长度错误，无法启动数字配对".into();
                        return Task::none();
                    }
                },
                Err(_) => {
                    state.status = "本机设备身份格式错误，无法启动数字配对".into();
                    return Task::none();
                }
            };
            let identity = pairing_crypto::DeviceIdentity::from_parts(device_id, signing_seed);
            let config = host::HostConfig {
                server: state.server.clone(), relay_token: state.relay_token.clone(),
                secret: state.secret, running: state.running.clone(),
                allow_input: state.allow_input.clone(),
                allow_clipboard: state.allow_clipboard.clone(),
                monitor_index: state.monitor_index.clone(),
                lan_direct: state.lan_direct.clone(),
                lan_active: state.lan_active.clone(),
                allow_udp_probe: state.allow_udp_probe.clone(),
                allow_udp_preview: state.allow_udp_preview.clone(),
                udp_active: state.udp_active.clone(),
                h264_enabled: state.h264_enabled.clone(),
                target_fps: state.target_fps.clone(),
                sent_frames: state.sent_frames.clone(),
                dropped_frames: state.dropped_frames.clone(),
                messages: state.tx.clone(),
            };
            state.running.store(true, Ordering::SeqCst);
            state.device_number.clear();
            state.pairing_code.clear();
            state.pairing_expires_at = 0;
            state.pending_pairing_request = None;
            state.status = "正在向中转服务器注册本机设备…".into();
            let (decision_tx, decision_rx) = mpsc::channel();
            state.pairing_decision_tx = Some(decision_tx);
            host_pairing::start(config, identity, decision_rx);
        }
        Msg::Stop => {
            tracing::info!("stopping local host and numeric pairing");
            state.running.store(false, Ordering::SeqCst);
            state.pairing_code.clear();
            state.pairing_expires_at = 0;
            state.pending_pairing_request = None;
            #[cfg(windows)]
            { state.pairing_decision_tx = None; }
            state.status = "正在撤销动态验证码并断开远程服务…".into();
        }
        Msg::NewKey if !state.running.load(Ordering::SeqCst) => {
            state.secret = protocol::new_secret();
            state.trust_saved = false;
            state.launch_on_login = false;
            state.unattended_input = false;
            let old = trusted_profile::revoke();
            let startup = trusted_profile::set_login_start(false);
            state.status = if old.is_ok() && startup.is_ok() {
                "已生成新密钥并撤销此前保存的可信配对".into()
            } else {
                "密钥已更新，但清理旧绑定失败；请检查用户启动项与加密配置".into()
            };
        },
        Msg::Tick => {
            #[cfg(windows)]
            {
                if !state.tray_initialized {
                    state.tray_initialized = true;
                    match tray::Tray::new() {
                        Ok(tray) => {
                            state.tray = Some(tray);
                            tracing::info!("Windows system tray initialized");
                        }
                        Err(error) => {
                            state.status = format!("系统托盘初始化失败，窗口将保持可见：{error}");
                            tracing::error!(error = %error, "system tray initialization failed");
                        }
                    }
                }

                if state.hide_on_startup && state.tray.is_some() {
                    state.hide_on_startup = false;
                    return window::get_latest().map(Msg::HideWindow);
                }

                let tray_actions = state.tray.as_ref()
                    .map(tray::Tray::drain_actions)
                    .unwrap_or_default();
                for action in tray_actions {
                    match action {
                        tray::Action::Open => {
                            return window::get_latest().map(Msg::ShowWindow);
                        }
                        tray::Action::ToggleHost => {
                            let is_running = state.running.load(Ordering::SeqCst);
                            return update(state, if is_running { Msg::Stop } else { Msg::Start });
                        }
                        tray::Action::ToggleInput => {
                            if state.allow_input.load(Ordering::SeqCst) {
                                state.tray_input_paused = true;
                                state.allow_input.store(false, Ordering::SeqCst);
                                state.status = "已从系统托盘暂停远程输入".into();
                            } else if state.tray_input_paused {
                                state.tray_input_paused = false;
                                state.allow_input.store(true, Ordering::SeqCst);
                                state.status = "已恢复此前由本机用户授权的远程输入".into();
                            } else {
                                state.status = "远程输入尚未授权；请在主窗口中明确授权".into();
                            }
                        }
                        tray::Action::Exit => {
                            if state.viewer_online {
                                let _ = state.viewer_tx.send(serde_json::json!({
                                    "type":"up", "x":state.viewer_pointer.0, "y":state.viewer_pointer.1
                                }).to_string());
                            }
                            state.running.store(false, Ordering::SeqCst);
                            state.viewer_active.store(false, Ordering::SeqCst);
                            state.allow_input.store(false, Ordering::SeqCst);
                            state.allow_clipboard.store(false, Ordering::SeqCst);
                            tracing::info!("full exit requested from tray menu");
                            return window::get_latest().map(Msg::ExitWindow);
                        }
                    }
                }
            }

            while let Ok(event) = state.viewer_rx.try_recv() {
                match event {
                    viewer::ViewerEvent::State(s) => state.viewer_status = s,
                    viewer::ViewerEvent::Connected => {
                        state.viewer_online = true;
                        state.viewer_wait_since = Some(Instant::now());
                        state.viewer_status = "WSS 已连接，正在等待有权限的 Windows 主机画面".into();
                    }
                    viewer::ViewerEvent::Disconnected => {
                        state.viewer_online = false;
                        state.viewer_wait_since = None;
                        state.viewer_pointer_valid = false;
                        state.viewer_active.store(false, Ordering::SeqCst);
                        if !state.viewer_status.contains("失败") && !state.viewer_status.contains("错误") {
                            state.viewer_status = "远程连接已断开，可直接再次连接".into();
                        }
                    }
                    viewer::ViewerEvent::Jpeg(bytes,width,height) => {
                        let first_frame = state.viewer_frame.is_none();
                        state.viewer_wait_since = None;
                        state.viewer_frame_size = (width,height);
                        state.viewer_frame = Some(image::Handle::from_bytes(bytes));
                        if first_frame {
                            state.viewer_status = "正在显示远程桌面 · JPEG 已收到".into();
                        }
                    },
                    viewer::ViewerEvent::Monitors(value) => {
                        state.viewer_monitor_info = viewer_geometry::Monitors::parse(&value);
                        let input_info = serde_json::from_str::<serde_json::Value>(&value).ok()
                            .and_then(|json| json.get("allow_input").and_then(|v| v.as_bool()));
                        state.viewer_monitors = match state.viewer_monitor_info {
                            Some(info) => format!("远程显示器 {}/{} · 输入权限：{}", info.selected+1, info.count,
                                match input_info {
                                    Some(true) => "已授权",
                                    Some(false) => "未授权（请在被控电脑安全与画质中开启）",
                                    None => "等待主机报告",
                                }),
                            None => "远程显示器元数据无效".into(),
                        };
                    },
                    viewer::ViewerEvent::Clipboard(value) => state.viewer_clipboard = value,
                }
            }
            if state.viewer_online && state.viewer_frame.is_none()
                && state.viewer_wait_since.is_some_and(|since| since.elapsed() > Duration::from_secs(10))
            {
                state.viewer_status = "WSS 已连接但未收到画面：请确认被控电脑已启动服务、配对密钥一致，并更新为 V0.13.2".into();
            }
            while let Ok(message) = state.rx.try_recv() {
                if message.starts_with("[STUN]") {
                    state.diagnostics = message.trim_start_matches("[STUN] ").to_string();
                    continue;
                }
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&message) {
                    match event.get("event").and_then(|v| v.as_str()).unwrap_or("") {
                        "pairing_device" => {
                            state.device_number = event.get("device_number").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("本机设备已注册").to_owned();
                        }
                        "pairing_code" => {
                            state.device_number = event.get("device_number").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                            state.pairing_code = event.get("code").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                            state.pairing_expires_at = event.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("动态验证码已更新").to_owned();
                        }
                        "pairing_request" => {
                            state.pending_pairing_request = Some(PairingRequestPrompt {
                                request_id: event.get("request_id").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
                                viewer_fingerprint: event.get("viewer_fingerprint").and_then(|v| v.as_str()).unwrap_or("未知设备").to_owned(),
                            });
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("收到配对请求").to_owned();
                        }
                        "pairing_request_expired" | "pairing_request_denied" => {
                            state.pending_pairing_request = None;
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("配对请求已结束").to_owned();
                        }
                        "pairing_approved" => {
                            state.pending_pairing_request = None;
                            state.pairing_code.clear();
                            state.pairing_expires_at = 0;
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("配对已批准").to_owned();
                        }
                        "pairing_stopped" => {
                            state.pairing_code.clear();
                            state.pairing_expires_at = 0;
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("数字配对已停止").to_owned();
                        }
                        "pairing_status" => {
                            state.status = event.get("message").and_then(|v| v.as_str()).unwrap_or("数字配对状态更新").to_owned();
                        }
                        _ => state.status = message,
                    }
                } else {
                    state.status = message;
                }
            }
        }
        _ => {}
    }
    Task::none()
}

fn card<'a>(child: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    container(child).padding(24).width(Length::Fill)
      .style(|_| container::Style::default()
           .background(Color::from_rgb8(29, 32, 39))
           .border(Border { color: Color::from_rgb8(55, 59, 67), width: 1.0, radius: 14.0.into() }))
      .into()
}
fn heading<'a>(title: &'a str, subtitle: &'a str) -> Element<'a, Msg> {
    column![text(title).size(22), text(subtitle).size(13).color(Color::from_rgb8(153, 164, 185))].spacing(6).into()
}
fn view(state: &App) -> Element<'_, Msg> {
    let active = state.running.load(Ordering::SeqCst);
    let (page_title, page_subtitle) = match state.page {
        Page::Home => ("设备概览", "查看本机设备、共享状态和最近使用的功能"),
        Page::Remote => ("远程控制", "连接另一台电脑，查看桌面并进行安全操作"),
        Page::Settings => ("设置", "管理远程权限、画质和可信设备"),
        Page::Network => ("连接设置", "配置中转服务并诊断网络连接"),
    };
    let (status_label, status_color) = if state.page == Page::Remote {
        if state.viewer_online {
            ("●  远程控制已连接", Color::from_rgb8(22, 75, 67))
        } else {
            ("○  尚未连接远程电脑", Color::from_rgb8(53, 63, 80))
        }
    } else if active {
        ("●  本机共享已启动", Color::from_rgb8(22, 75, 67))
    } else {
        ("○  本机共享未启动", Color::from_rgb8(53, 63, 80))
    };
    let top = row![
        column![text("P2P REMOTE  /  V0.14").size(13).color(Color::from_rgb8(255,177,115)),
                text(page_title).size(28),
                text(page_subtitle).size(13).color(Color::from_rgb8(156,168,189))].spacing(9),
        horizontal_space(),
        container(text(status_label).size(14))
            .padding([12,20])
            .style(move |_| container::Style::default().background(status_color)
                .border(Border {color: Color::TRANSPARENT, width: 0.0, radius: 22.into()}))
    ].align_y(iced::Alignment::Center);

    let connection = card(column![
        heading("连接与服务器配置", "高级网络参数仅在此页面管理"),
        text("配置你的中转服务器和当前兼容配对凭据。数字验证码配对接入完成前，旧密钥连接仍保留。").size(13),
        text("中转服务器地址").size(13),
        text_input("wss://p2ptest.aikissgo.com", &state.server).on_input(Msg::Server).padding(13).size(15),
        text("中转访问口令（与 VPS 的 RELAY_ACCESS_TOKEN 一致）").size(13),
        text_input("输入服务器访问口令", &state.relay_token).on_input(Msg::Token).secure(true).padding(13).size(15),
        row![
            text(format!("本机设备号：{}", protocol::device_number(&state.secret)))
                .size(19).color(Color::from_rgb8(255, 177, 115)),
            button("复制设备号").on_press(Msg::CopyDeviceNumber).padding(10)
        ].spacing(14).align_y(iced::Alignment::Center),
        text("数字设备号仅用于识别；不能代替加密配对密钥。手机可粘贴密钥直接连接。").size(12)
            .color(Color::from_rgb8(156, 168, 189)),
        text("配对密钥仅向已授权的 Windows / Android 控制端分享").size(13),
        row![
            container(text(if state.show_session_key {
                hex::encode(state.secret)
            } else {
                "•••• •••• •••• •••• · 点击显示后可用于配对".to_owned()
            }).size(13))
                .padding(13).width(Length::Fill)
                .style(|_| container::Style::default()
                    .background(Color::from_rgb8(19, 21, 27))
                    .border(Border {color:Color::from_rgb8(60,74,102),width:1.0,radius:9.into()})),
            button(if state.show_session_key { "隐藏密钥" } else { "显示密钥" })
                .on_press(Msg::ToggleSessionKey).padding(12),
            button("复制配对密钥").on_press(Msg::CopyPairingKey).padding(12)
        ].spacing(8).align_y(iced::Alignment::Center),
        row![button("生成新密钥并撤销旧配对").on_press_maybe((!active).then_some(Msg::NewKey)).padding(13),
             horizontal_space(),
             if active {button("停止远程服务").on_press(Msg::Stop).padding(13)}
             else {button("启动远程服务").on_press(Msg::Start).padding(13)}]
            .align_y(iced::Alignment::Center),
    ].spacing(15));

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let code_seconds_left = state.pairing_expires_at.saturating_sub(now_unix).max(0);
    let device_number_display = if state.device_number.is_empty() { "启动共享后自动分配" } else { state.device_number.as_str() };
    let pairing_code_display = if state.pairing_code.is_empty() { "等待生成" } else { state.pairing_code.as_str() };
    let home_overview = card(column![
        heading("本机设备", "服务器分配的设备号和限时配对验证码"),
        row![
            column![
                text("9 位设备号").size(13).color(Color::from_rgb8(153, 164, 185)),
                text(device_number_display).size(32)
                    .color(Color::from_rgb8(91, 210, 225)),
                text("设备号只用于定位设备，不是访问凭据。").size(12)
                    .color(Color::from_rgb8(153, 164, 185))
            ].spacing(7),
            horizontal_space(),
            button("复制设备号").on_press(Msg::CopyDeviceNumber).padding(12)
        ].align_y(iced::Alignment::Center),
        row![
            column![
                text("6 位动态验证码").size(13).color(Color::from_rgb8(153, 164, 185)),
                text(pairing_code_display).size(30).color(Color::from_rgb8(245, 190, 108)),
                text(if code_seconds_left > 0 { format!("剩余 {} 秒 · 使用一次后立即失效", code_seconds_left) } else { "有效期 120 秒 · 等待共享服务启动".into() })
                    .size(12).color(Color::from_rgb8(153, 164, 185))
            ].spacing(7),
            horizontal_space(),
            button("复制验证码").on_press(Msg::CopyPairingCode).padding(12)
        ].align_y(iced::Alignment::Center),
        row![
            container(text(if active { "● 配对服务运行中" } else { "○ 配对服务未启动" }).size(14))
                .padding([10,14])
                .style(move |_| container::Style::default()
                    .background(if active { Color::from_rgb8(24, 70, 61) } else { Color::from_rgb8(48, 56, 72) })
                    .border(Border { color: Color::TRANSPARENT, width: 0.0, radius: 18.into() })),
            horizontal_space(),
            if active {
                button("停止共享").on_press(Msg::Stop).padding(12)
            } else {
                button("启动共享").on_press(Msg::Start).padding(12)
            }
        ].align_y(iced::Alignment::Center),
        text(&state.status).size(13).color(Color::from_rgb8(166, 178, 197))
    ].spacing(17));

    let pairing_approval: Element<'_, Msg> = if let Some(request) = &state.pending_pairing_request {
        card(column![
            heading("新的远程连接请求", "验证码校验已通过，请确认是否允许该设备连接"),
            text(format!("请求设备指纹：{}", request.viewer_fingerprint)).size(14),
            row![
                button("拒绝").on_press(Msg::DenyPairing).padding(12),
                button("允许连接").on_press(Msg::ApprovePairing).padding(12)
            ].spacing(10)
        ].spacing(12))
    } else {
        column![].into()
    };

    let quick_connect = card(column![
        heading("远程连接", "从另一台 Windows 或 Android 设备访问已授权电脑"),
        row![
            column![
                text("远程桌面").size(18),
                text("查看屏幕、缩放画面、操作鼠标键盘，并支持中文文字输入。")
                    .size(13).color(Color::from_rgb8(153, 164, 185))
            ].spacing(6),
            horizontal_space(),
            button("打开远程控制").on_press(Msg::Navigate(Page::Remote)).padding(13)
        ].align_y(iced::Alignment::Center),
        text("首次配对必须由被控电脑用户明确授权。隐藏窗口不会自动开放控制权限。")
            .size(12).color(Color::from_rgb8(204, 178, 130))
    ].spacing(14));

    let direct = card(column![
        heading("02 / 局域网 P2P 直连", "LAN 同一子网优先，手机直连失败会自动回退 VPS"),
        checkbox("允许局域网加密直连（TCP 43821，仅当前远控会话有效）",
            state.lan_direct.load(Ordering::SeqCst))
            .on_toggle(Msg::Lan).size(16),
        text("该选项仅在停止服务时可修改；不会创建 Windows 防火墙规则。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        text("手机输入此电脑的私有 IPv4 地址（Windows ipconfig 查看）。仅 RFC1918 网段可访问。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("局域网直连采用加密握手认证，访问口令不会通过 ws:// 私网链接发送。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("本版可探测 STUN 出口地址，但未实现 ICE/UDP 打洞；5G 跨网仍优先 VPS。").size(13)
            .color(Color::from_rgb8(204,178,130)),
    ].spacing(13));
    let safety = card(column![
        heading("03 / 权限与隐私", "当前会话权限可以随时关闭；持久化无人值守授权必须单独明确选择"),
        checkbox("允许已配对设备操作鼠标、键盘与聊天文字（Windows / Android）", state.allow_input.load(Ordering::SeqCst)).on_toggle(Msg::Allow).size(16),
        checkbox("允许 Android 端按需读写 Windows 剪贴板文字", state.allow_clipboard.load(Ordering::SeqCst)).on_toggle(Msg::Clipboard).size(16),
        text("默认关闭剪贴板授权；不后台读取，不自动同步。每次传递由手机端手动触发。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        text("端到端 AES-256-GCM · 临时配对密钥 · 中转服务器无法解密屏幕画面").size(13).color(Color::from_rgb8(163,178,205)),
        text("仅当前登录的 Windows 用户桌面。UAC、安全桌面、锁屏、系统服务均不支持。").size(13).color(Color::from_rgb8(204,178,130)),
    ].spacing(15));
    let trust = card(column![
        heading("04 / 可信设备配对", "单台 Android 设备记住凭据；Windows DPAPI 加密、仅当前系统用户可恢复"),
        text(format!("设备指纹：{}", &protocol::room_id(&state.secret)[..12])).size(15),
        checkbox("我同意 Windows 用户登录后自动启动并连接 VPS", state.launch_on_login)
            .on_toggle(Msg::LoginStart).size(16),
        checkbox("我同意下次自动连接后允许已配对手机控制鼠标/键盘", state.unattended_input)
            .on_toggle(Msg::UnattendedInput).size(16),
        text("勾选后须点击加密保存才会生效；托盘隐藏和开机前控制尚不支持。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        row![
            button("加密保存可信配对").on_press(Msg::SaveTrust).padding(12),
            button("撤销所有已记住的配对").on_press(Msg::RevokeTrust).padding(12)
        ].spacing(12),
        text(if state.trust_saved {"● 已在此 Windows 用户中保存加密配对"}
            else {"○ 尚未保存可信配对"}).size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("此版使用单会话密钥；撤销会轮换密钥，使旧手机失效，不提供逐设备公钥管理。").size(13)
            .color(Color::from_rgb8(158,172,199)),
    ].spacing(15));
    let video = card(column![
        heading("05 / 画质与网络", "优先验证真实 H.264 链路；默认 JPEG，手动打开软件 H.264"),
        checkbox("启用 H.264 软件编码（Android MediaCodec 解码）",
            state.h264_enabled.load(Ordering::SeqCst)).on_toggle(Msg::H264).size(16),
        text("H.264 软件编码最高 1280×720；JPEG 默认约 6 FPS，H.264 帧率可选。").size(13)
            .color(Color::from_rgb8(163,178,205)),
        row![
            button("节省带宽 · 6 FPS").on_press(Msg::Fps(6)).padding(10),
            button("标准 · 12 FPS").on_press(Msg::Fps(12)).padding(10),
            button("流畅 · 20 FPS").on_press(Msg::Fps(20)).padding(10)
        ].spacing(10),
        text(format!(
            "当前请求 {} FPS · 已发送 {} 帧 · 队列丢帧 {}",
            state.target_fps.load(Ordering::SeqCst),
            state.sent_frames.load(Ordering::Relaxed),
            state.dropped_frames.load(Ordering::Relaxed)
        )).size(14),
        text("数值只是编码请求与丢帧统计，不代表实测 FPS；本版不具备 GPU 硬件编码。").size(12)
            .color(Color::from_rgb8(204,178,130)),
    ].spacing(15));
    let status = card(column![
        heading("06 / 运行状态", "实时连接状态 · 自动断线重连（最长等待 30 秒）"),
        text(&state.status).size(18),
        row![
            text(format!(
                "当前显示器序号：{} · {}",
                state.monitor_index.load(Ordering::SeqCst) + 1,
                if state.lan_active.load(Ordering::SeqCst) {
                    "正在使用局域网直连，已暂停向 VPS 上传视频"
                } else { "局域网未连接，使用 VPS 或等待连接" }
            )).size(14),
            button("切换到下一显示器").on_press(Msg::NextMonitor).padding(10)
        ].spacing(14).align_y(iced::Alignment::Center),
        text("Android V0.7：同网段优先填写局域网 IPv4；连接失败回退 VPS。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("仅在点击加密保存后，本机才以当前 Windows 用户 DPAPI 保护口令与配对密钥。").size(13)
            .color(Color::from_rgb8(158,172,199)),
    ].spacing(14));

    let diagnostics = card(column![
        heading("07 / 公网 UDP/STUN 诊断", "通过你自己的 VPS UDP 3478 查看 NAT 出口地址"),
        button("探测公网 UDP 映射").on_press(Msg::ProbeNetwork).padding(12),
        text(&state.diagnostics).size(14),
        text("STUN 探测使用独立短期 UDP Socket：不交换真实候选地址，也不传输密钥。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        text("V0.9 通过加密候选交换尝试真实 UDP 双端握手；画面仍通过 VPS WSS。").size(13)
            .color(Color::from_rgb8(158,172,199)),
    ].spacing(14));

    let udp = card(column![
        heading("08 / 公网 UDP 双端连通性", "Android 主动发起，电脑明确允许后才尝试"),
        checkbox("允许已配对 Android 发起 UDP 直连测试（不会传输屏幕画面）",
            state.allow_udp_probe.load(Ordering::SeqCst))
            .on_toggle(Msg::AllowUdpProbe).size(16),
        text("请在启动服务前勾选。手机连接 VPS 后点击“测试 UDP 直连”；本机无须手动开放 UDP 端口。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("双方候选地址仅在端到端加密 WSS 消息中交换；UDP 只传输加密挑战响应。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("若看到“UDP 握手成功”，仅说明 12 秒内双方可互发加密探测包，并非视频已切换。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        checkbox("额外允许已验证 UDP 链路传输 20 秒低画质 JPEG 预览（默认关闭）",
            state.allow_udp_preview.load(Ordering::SeqCst))
            .on_toggle(Msg::AllowUdpPreview).size(16),
        text(format!("当前 UDP 预览：{}",
            if state.udp_active.load(Ordering::SeqCst) { "正在传输，WSS 视频暂缓" }
            else { "未运行，WSS 正常传输" }
        )).size(13),
        text("只有 Android 主动请求且双方通过加密验证时，才传输640×360 JPEG预览。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("未完成全部 UDP 硬件兼容测试，不包含 UDP 远程输入。超时自动切回 WSS。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        text("默认关闭。需要 VPS 已启用 STUN UDP 3478；不支持完整 ICE/TURN 或 QUIC 视频。").size(13)
            .color(Color::from_rgb8(204,178,130)),
    ].spacing(13));


    // The fullscreen viewer is a genuine OS fullscreen window. Size is tracked
    // from iced resize events; both image and pointer use the same viewport.
    let (viewer_width, viewer_height) = viewer_geometry::viewer_dimensions(
        state.viewer_fullscreen, state.window_size, state.viewer_scale.dimensions()
    );
    let remote_image: Element<'_, Msg> = if let Some(handle) = &state.viewer_frame {
        mouse_area(image(handle.clone())
                .width(Length::Fixed(viewer_width))
                .height(Length::Fixed(viewer_height)))
            .on_move(Msg::ViewerMove)
            .on_press(Msg::ViewerDown)
            .on_release(Msg::ViewerUp)
            .into()
    } else {
        container(text("等待已授权的远程 Windows 桌面画面").size(15)
            .color(Color::from_rgb8(165, 177, 190)))
            .width(Length::Fixed(viewer_width)).height(Length::Fixed(viewer_height))
            .center_x(Length::Fill).center_y(Length::Fill)
            .style(|_| container::Style::default()
                .background(Color::from_rgb8(19, 21, 27))
                .border(Border { color: Color::from_rgb8(55, 59, 67),
                    width: 1.0, radius: 12.0.into() }))
            .into()
    };
    // The host reports 1–16 screens. Prev/next work for every screen;
    // show quick-select buttons for the first six without overflowing.
    let monitor_bar: Element<'_, Msg> = if let Some(info) = state.viewer_monitor_info {
        let mut chooser = row![text(format!("显示器 {}/{}", info.selected+1,info.count)).size(13)]
            .spacing(7).align_y(iced::Alignment::Center);
        for index in 0..info.count.min(6) {
            chooser = chooser.push(button(text(format!("{}", index+1))).padding(8)
                .on_press_maybe(state.viewer_online.then_some(Msg::ViewerMonitor(index))));
        }
        chooser.into()
    } else {
        row![text("等待远程显示器信息").size(13)].into()
    };
    let viewer_panel: Element<'_, Msg> = if state.viewer_fullscreen {
        container(column![
            row![
                text("P2P REMOTE  /  远程全屏").size(17),
                text(&state.viewer_status).size(13),
                horizontal_space(),
                button("退出全屏").on_press(Msg::ToggleViewerFullscreen).padding(10),
                button("断开连接").on_press(Msg::ViewerDisconnect).padding(10)
            ].spacing(12).align_y(iced::Alignment::Center),
            remote_image,
            row![
                monitor_bar,
                horizontal_space(),
                button("上一屏").on_press_maybe(state.viewer_monitor_info
                    .filter(|_|state.viewer_online).map(|m|Msg::ViewerMonitor(m.previous()))),
                button("下一屏").on_press_maybe(state.viewer_monitor_info
                    .filter(|_|state.viewer_online).map(|m|Msg::ViewerMonitor(m.next())))
            ].spacing(12).align_y(iced::Alignment::Center),
            row![
                text("快捷操作").size(12),
                button("右键").on_press_maybe((state.viewer_online && state.viewer_pointer_valid).then_some(Msg::ViewerRight)),
                button("↑").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("up"))),
                button("↓").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("down"))),
                button("←").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("left"))),
                button("→").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("right"))),
                button("Esc").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("escape")))
            ].spacing(8).align_y(iced::Alignment::Center)
        ].spacing(12)).padding(16).width(Length::Fill).into()
    } else {
        card(column![
            heading("远程控制", "通过加密 WSS 连接另一台已授权的 Windows 电脑"),
            text("双方需使用相同 Relay Token，远程 PC Host 必须启动并授权控制").size(13),
            text_input("输入远程 PC 的 64 位会话密钥", &state.viewer_key)
                .on_input(Msg::ViewerKey).secure(true).padding(12).size(15),
            row![
                button("连接远程 Windows")
                    .on_press_maybe((!state.viewer_active.load(Ordering::SeqCst)).then_some(Msg::ViewerConnect)),
                button("断开 Viewer").on_press(Msg::ViewerDisconnect),
                button("进入全屏").on_press(Msg::ToggleViewerFullscreen)
            ].spacing(10),
            text(&state.viewer_status).size(14)
                .color(if state.viewer_online { Color::from_rgb8(98, 208, 166) }
                    else { Color::from_rgb8(181, 190, 204) }),
            row![
                text("画面大小").size(13),
                button("紧凑").on_press(Msg::ViewerZoom(ViewerScale::Compact)).padding(9),
                button("标准").on_press(Msg::ViewerZoom(ViewerScale::Standard)).padding(9),
                button("放大").on_press(Msg::ViewerZoom(ViewerScale::Large)).padding(9),
                text(format!("{} × {}", viewer_width as u32, viewer_height as u32))
                    .size(12).color(Color::from_rgb8(153, 164, 185))
            ].spacing(9).align_y(iced::Alignment::Center),
            text("Viewer 将自动请求 JPEG 兼容画面；无需手动更改被控端 H.264 设置。").size(13),
            remote_image,
            row![
                monitor_bar,
                button("上一屏").on_press_maybe(state.viewer_monitor_info
                    .filter(|_|state.viewer_online).map(|m|Msg::ViewerMonitor(m.previous()))),
                button("下一屏").on_press_maybe(state.viewer_monitor_info
                    .filter(|_|state.viewer_online).map(|m|Msg::ViewerMonitor(m.next())))
            ].spacing(9).align_y(iced::Alignment::Center),
            row![
                button("右键").on_press_maybe((state.viewer_online && state.viewer_pointer_valid).then_some(Msg::ViewerRight)),
                button("上滚").on_press_maybe(state.viewer_online.then_some(Msg::ViewerScroll(-3))),
                button("下滚").on_press_maybe(state.viewer_online.then_some(Msg::ViewerScroll(3)))
            ].spacing(9),
            row![
                text_input("向远程 Windows 输入文字", &state.viewer_text)
                    .on_input(Msg::ViewerText).padding(10),
                button("发送文字").on_press_maybe(state.viewer_online.then_some(Msg::ViewerSendText))
            ].spacing(9),
            row![
                button("↑").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("up"))),
                button("↓").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("down"))),
                button("←").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("left"))),
                button("→").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("right"))),
                button("Home").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("home"))),
                button("End").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("end"))),
                button("PgUp").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("page_up"))),
                button("PgDn").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("page_down")))
            ].spacing(8),
            row![
                button("Enter").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("enter"))),
                button("Backspace").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("backspace"))),
                button("Tab").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("tab"))),
                button("Esc").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("escape"))),
                button("Delete").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("delete"))),
                button("空格").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("space")))
            ].spacing(8),
            row![
                button("全选").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_a"))),
                button("复制").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_c"))),
                button("粘贴").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_v"))),
                button("剪切").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_x"))),
                button("撤销").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_z"))),
                button("重做").on_press_maybe(state.viewer_online.then_some(Msg::ViewerKeyPress("ctrl_y")))
            ].spacing(8),
            row![
                button("读取远程剪贴板").on_press_maybe(state.viewer_online.then_some(Msg::ViewerClipboardGet)),
                button("将输入文字送入远程剪贴板").on_press_maybe(state.viewer_online.then_some(Msg::ViewerClipboardSet))
            ].spacing(9),
            text(&state.viewer_clipboard).size(12),
            text(&state.viewer_monitors).size(12),
            text("快捷键仅在 Host 明确允许远程输入时生效，不支持 UAC 和安全桌面。").size(12)
        ].spacing(12))
    };

    // Sunlogin-inspired information architecture, with original P2P branding:
    // one primary task per screen instead of ten stacked configuration panels.
    let page_content: Element<'_, Msg> = match state.page {
        Page::Home => column![home_overview, pairing_approval, quick_connect, trust].spacing(18).into(),
        Page::Remote => column![viewer_panel].spacing(18).into(),
        Page::Settings => column![safety, video, direct].spacing(18).into(),
        Page::Network => column![connection, diagnostics, udp].spacing(18).into(),
    };
    let menu = column![
        text("P2P").size(29).color(Color::from_rgb8(255, 177, 115)),
        text("REMOTE DESKTOP").size(11).color(Color::from_rgb8(166, 174, 187)),
        container(text("工作空间").size(12).color(Color::from_rgb8(130, 140, 151))).padding([22, 0]),
        button(if state.page == Page::Home { "●  我的设备" } else { "    我的设备" })
            .on_press(Msg::Navigate(Page::Home)).width(Length::Fill).padding(14),
        button(if state.page == Page::Remote { "●  远程控制" } else { "    远程控制" })
            .on_press(Msg::Navigate(Page::Remote)).width(Length::Fill).padding(14),
        button(if state.page == Page::Settings { "●  设置与权限" } else { "    设置与权限" })
            .on_press(Msg::Navigate(Page::Settings)).width(Length::Fill).padding(14),
        button(if state.page == Page::Network { "●  连接设置" } else { "    连接设置" })
            .on_press(Msg::Navigate(Page::Network)).width(Length::Fill).padding(14),
        vertical_space(),
        text(if active { "● 本机共享服务已启动" } else { "○ 本机共享服务未启动" })
            .size(13)
            .color(if active { Color::from_rgb8(98, 208, 166) } else { Color::from_rgb8(166, 174, 187) }),
        text(format!("中转：{}", state.server)).size(11).color(Color::from_rgb8(130, 140, 151)),
    ].spacing(10);
    let side = container(menu).width(Length::Fixed(224.0)).height(Length::Fill).padding(22)
        .style(|_| container::Style::default()
            .background(Color::from_rgb8(21, 23, 29))
            .border(Border {color: Color::from_rgb8(46, 49, 57), width: 1.0, radius: 0.0.into()}));
    if state.viewer_fullscreen {
        // The fullscreen workspace has its own status/exit bar; do not duplicate
        // the normal top banner and steal space from remote video.
        return container(scrollable(
            container(page_content).padding(8)
        )).width(Length::Fill).height(Length::Fill)
            .center_x(Length::Fill).into();
    }
    let body = scrollable(
        container(column![top, page_content].spacing(22)).padding(24).width(Length::Fill)
    ).width(Length::Fill).height(Length::Fill);
    container(row![side, body].height(Length::Fill))
        .width(Length::Fill).height(Length::Fill).into()
}

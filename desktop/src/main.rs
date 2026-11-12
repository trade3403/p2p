mod host;
mod protocol;
mod reconnect;
mod local_settings;
mod remote_ops;
mod trusted_profile;
mod video;
mod lan;

use iced::{widget::{button, checkbox, column, container, horizontal_space, row, scrollable, text, text_input}, Border, Color, Element, Length, Subscription, Theme};
use std::{sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc, mpsc::{self, Receiver, Sender}}, time::Duration};

fn main() -> iced::Result {
    iced::application("P2P Remote · 个人远程桌面", update, view)
        .theme(|_| Theme::TokyoNight)
        .subscription(|_| Subscription::batch([iced::time::every(Duration::from_millis(250)).map(|_| Msg::Tick)]))
        .window_size((1080.0, 740.0))
        .run()
}

struct App {
    server: String,
    relay_token: String,
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
    h264_enabled: Arc<AtomicBool>,
    target_fps: Arc<AtomicU32>,
    sent_frames: Arc<AtomicU64>,
    dropped_frames: Arc<AtomicU64>,
    status: String,
    tx: Sender<String>,
    rx: Receiver<String>,
}
impl Default for App {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        // Failed DPAPI decryption never silently recreates a trusted session.
        let trusted = trusted_profile::load().ok().flatten();
        let secret = trusted.as_ref()
            .and_then(|p| hex::decode(&p.session_hex).ok())
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .unwrap_or_else(protocol::new_secret);
        let mut app = Self {
            server: trusted.as_ref().map(|p| p.relay_url.clone())
                .unwrap_or_else(local_settings::load_relay_url),
            relay_token: trusted.as_ref().map(|p| p.relay_token.clone()).unwrap_or_default(),
            secret,
            running: Arc::new(AtomicBool::new(false)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(false)),
            lan_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(12)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            launch_on_login: trusted.as_ref().map(|p| p.launch_on_login).unwrap_or(false),
            unattended_input: trusted.as_ref().map(|p| p.unattended_input).unwrap_or(false),
            trust_saved: trusted.is_some(),
            status: "未连接 · 手动启动或启用登录后自动连接".into(),
            tx,
            rx,
        };
        // Auto-start is opt-in and only runs after normal user logon.
        // It is never a Windows service and never attempts secure-desktop capture.
        if std::env::args().any(|arg| arg == "--autostart")
            && app.trust_saved && app.launch_on_login
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
// Closing the visible window must terminate the session; never leave a
// hidden capture thread connected after the owner exits the application.
impl Drop for App {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.allow_input.store(false, Ordering::SeqCst);
        self.allow_clipboard.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
enum Msg { Server(String), Token(String), Allow(bool), Clipboard(bool), NextMonitor,
    LoginStart(bool), UnattendedInput(bool), SaveTrust, RevokeTrust,
    H264(bool), Fps(u32), Lan(bool), Start, Stop, NewKey, Tick }

fn update(state: &mut App, msg: Msg) {
    match msg {
        Msg::Server(s) if !state.running.load(Ordering::SeqCst) => state.server = s,
        Msg::Token(s) if !state.running.load(Ordering::SeqCst) => state.relay_token = s,
        Msg::Allow(value) => state.allow_input.store(value, Ordering::SeqCst),
        Msg::Clipboard(value) => state.allow_clipboard.store(value, Ordering::SeqCst),
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
            if state.relay_token.trim().is_empty() &&
                !state.lan_direct.load(Ordering::SeqCst) {
                state.status = "请填写 VPS 口令或选择局域网直连".into(); return;
            }
            if let Err(err) = local_settings::save_relay_url(state.server.trim()) {
                state.status = format!("服务器地址无效或无法保存：{err}");
                return;
            }
            state.running.store(true, Ordering::SeqCst);
            state.status = "正在连接 VPS...".into();
            host::start(host::HostConfig {
                server: state.server.clone(), relay_token: state.relay_token.clone(),
                secret: state.secret, running: state.running.clone(),
                allow_input: state.allow_input.clone(),
                allow_clipboard: state.allow_clipboard.clone(),
                monitor_index: state.monitor_index.clone(),
                lan_direct: state.lan_direct.clone(),
                lan_active: state.lan_active.clone(),
                h264_enabled: state.h264_enabled.clone(),
                target_fps: state.target_fps.clone(),
                sent_frames: state.sent_frames.clone(),
                dropped_frames: state.dropped_frames.clone(),
                messages: state.tx.clone(),
            });
        }
        Msg::Stop => { state.running.store(false, Ordering::SeqCst); state.status = "正在断开...".into(); }
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
            while let Ok(message) = state.rx.try_recv() { state.status = message; }
        }
        _ => {}
    }
}

fn card<'a>(child: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    container(child).padding(24).width(Length::Fill)
      .style(|_| container::Style::default()
           .background(Color::from_rgb8(27, 34, 52))
           .border(Border { color: Color::from_rgb8(52, 64, 90), width: 1.0, radius: 16.0.into() }))
      .into()
}
fn heading<'a>(title: &'a str, subtitle: &'a str) -> Element<'a, Msg> {
    column![text(title).size(24), text(subtitle).size(13).color(Color::from_rgb8(153, 164, 185))].spacing(6).into()
}
fn view(state: &App) -> Element<'_, Msg> {
    let active = state.running.load(Ordering::SeqCst);
    let top = row![
        column![text("P2P  /  REMOTE").size(14).color(Color::from_rgb8(127,171,255)),
                text("个人远程桌面").size(33),
                text("Rust 原生客户端  ·  Windows Host  ·  V0.7 测试版").size(14).color(Color::from_rgb8(156,168,189))].spacing(9),
        horizontal_space(),
        container(text(if active {"●  服务已启动"} else {"○  尚未连接"}).size(14))
            .padding([12,20])
            .style(move |_| container::Style::default().background(if active {Color::from_rgb8(22,75,67)} else {Color::from_rgb8(53,63,80)}).border(Border {color: Color::TRANSPARENT, width: 0.0, radius: 22.into()}))
    ].align_y(iced::Alignment::Center);

    let connection = card(column![
        heading("01 / 连接设置", "局域网可选 P2P 直连；跨公网仍通过 VPS 的 WSS 中转"),
        text("中转服务器地址").size(13),
        text_input("wss://remote.example.com", &state.server).on_input(Msg::Server).padding(13).size(15),
        text("中转访问口令（与 VPS 的 RELAY_ACCESS_TOKEN 一致）").size(13),
        text_input("输入服务器访问口令", &state.relay_token).on_input(Msg::Token).secure(true).padding(13).size(15),
        text("端到端加密会话密钥（仅分享给你的 Android 手机）").size(13),
        container(text(hex::encode(state.secret)).size(14)).padding(13).width(Length::Fill)
            .style(|_| container::Style::default().background(Color::from_rgb8(17,25,42)).border(Border {color:Color::from_rgb8(60,74,102),width:1.0,radius:9.into()})),
        row![button("生成新密钥并撤销旧配对").on_press_maybe((!active).then_some(Msg::NewKey)).padding(13),
             horizontal_space(),
             if active {button("停止远程服务").on_press(Msg::Stop).padding(13)}
             else {button("启动远程服务").on_press(Msg::Start).padding(13)}]
            .align_y(iced::Alignment::Center),
    ].spacing(15));

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
        text("本版未实现 STUN/ICE 打洞；不同 Wi-Fi/5G 不一定能直连，仍需 VPS。").size(13)
            .color(Color::from_rgb8(204,178,130)),
    ].spacing(13));
    let safety = card(column![
        heading("03 / 权限与隐私", "当前会话权限可以随时关闭；持久化无人值守授权必须单独明确选择"),
        checkbox("允许 Android 端操作鼠标及输入文字", state.allow_input.load(Ordering::SeqCst)).on_toggle(Msg::Allow).size(16),
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

    container(scrollable(container(column![top, connection, direct, safety, trust, video, status].spacing(22)).width(Length::Fill).max_width(920).padding(22)))
        .width(Length::Fill).height(Length::Fill)
        .center_x(Length::Fill).into()
}

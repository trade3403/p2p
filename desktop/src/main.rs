mod host;
mod protocol;
mod reconnect;
mod local_settings;
mod remote_ops;
mod trusted_profile;

use iced::{widget::{button, checkbox, column, container, horizontal_space, row, scrollable, text, text_input}, Border, Color, Element, Length, Subscription, Theme};
use std::{sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, Arc, mpsc::{self, Receiver, Sender}}, time::Duration};

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
    Start, Stop, NewKey, Tick }

fn update(state: &mut App, msg: Msg) {
    match msg {
        Msg::Server(s) if !state.running.load(Ordering::SeqCst) => state.server = s,
        Msg::Token(s) if !state.running.load(Ordering::SeqCst) => state.relay_token = s,
        Msg::Allow(value) => state.allow_input.store(value, Ordering::SeqCst),
        Msg::Clipboard(value) => state.allow_clipboard.store(value, Ordering::SeqCst),
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
            if state.relay_token.trim().is_empty() {
                state.status = "必须填写 VPS RELAY_ACCESS_TOKEN".into(); return;
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
                text("Rust 原生客户端  ·  Windows Host  ·  V0.5 测试版").size(14).color(Color::from_rgb8(156,168,189))].spacing(9),
        horizontal_space(),
        container(text(if active {"●  服务已启动"} else {"○  尚未连接"}).size(14))
            .padding([12,20])
            .style(move |_| container::Style::default().background(if active {Color::from_rgb8(22,75,67)} else {Color::from_rgb8(53,63,80)}).border(Border {color: Color::TRANSPARENT, width: 0.0, radius: 22.into()}))
    ].align_y(iced::Alignment::Center);

    let connection = card(column![
        heading("01 / 连接设置", "VPS 使用 WSS 转发加密数据，家庭电脑不需要固定公网 IP"),
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

    let safety = card(column![
        heading("02 / 权限与隐私", "当前会话权限可以随时关闭；持久化无人值守授权必须单独明确选择"),
        checkbox("允许 Android 端操作鼠标及输入文字", state.allow_input.load(Ordering::SeqCst)).on_toggle(Msg::Allow).size(16),
        checkbox("允许 Android 端按需读写 Windows 剪贴板文字", state.allow_clipboard.load(Ordering::SeqCst)).on_toggle(Msg::Clipboard).size(16),
        text("默认关闭剪贴板授权；不后台读取，不自动同步。每次传递由手机端手动触发。").size(13)
            .color(Color::from_rgb8(204,178,130)),
        text("端到端 AES-256-GCM · 临时配对密钥 · 中转服务器无法解密屏幕画面").size(13).color(Color::from_rgb8(163,178,205)),
        text("仅当前登录的 Windows 用户桌面。UAC、安全桌面、锁屏、系统服务均不支持。").size(13).color(Color::from_rgb8(204,178,130)),
    ].spacing(15));
    let trust = card(column![
        heading("03 / 可信设备配对", "单台 Android 设备记住凭据；Windows DPAPI 加密、仅当前系统用户可恢复"),
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
    let status = card(column![
        heading("04 / 运行状态", "实时连接状态 · 自动断线重连（最长等待 30 秒）"),
        text(&state.status).size(18),
        row![
            text(format!("当前显示器序号：{}", state.monitor_index.load(Ordering::SeqCst) + 1)).size(14),
            button("切换到下一显示器").on_press(Msg::NextMonitor).padding(10)
        ].spacing(14).align_y(iced::Alignment::Center),
        text("Android V0.5：可选择使用 Keystore 保存一次性输入的可信配对。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("仅在点击加密保存后，本机才以当前 Windows 用户 DPAPI 保护口令与配对密钥。").size(13)
            .color(Color::from_rgb8(158,172,199)),
    ].spacing(14));

    container(scrollable(container(column![top, connection, safety, trust, status].spacing(22)).width(Length::Fill).max_width(920).padding(22)))
        .width(Length::Fill).height(Length::Fill)
        .center_x(Length::Fill).into()
}

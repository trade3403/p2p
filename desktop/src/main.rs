mod host;
mod protocol;
mod reconnect;
mod local_settings;

use iced::{widget::{button, checkbox, column, container, horizontal_space, row, scrollable, text, text_input}, Border, Color, Element, Length, Subscription, Theme};
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc, mpsc::{self, Receiver, Sender}}, time::Duration};

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
    status: String,
    tx: Sender<String>,
    rx: Receiver<String>,
}
impl Default for App {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            server: local_settings::load_relay_url(),
            relay_token: String::new(),
            secret: protocol::new_secret(),
            running: Arc::new(AtomicBool::new(false)),
            allow_input: Arc::new(AtomicBool::new(false)),
            status: "未连接 · 请配置 VPS 中转地址".into(), tx, rx,
        }
    }
}
#[derive(Debug, Clone)]
enum Msg { Server(String), Token(String), Allow(bool), Start, Stop, NewKey, Tick }

fn update(state: &mut App, msg: Msg) {
    match msg {
        Msg::Server(s) if !state.running.load(Ordering::SeqCst) => state.server = s,
        Msg::Token(s) if !state.running.load(Ordering::SeqCst) => state.relay_token = s,
        Msg::Allow(value) => state.allow_input.store(value, Ordering::SeqCst),
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
                allow_input: state.allow_input.clone(), messages: state.tx.clone(),
            });
        }
        Msg::Stop => { state.running.store(false, Ordering::SeqCst); state.status = "正在断开...".into(); }
        Msg::NewKey if !state.running.load(Ordering::SeqCst) => state.secret = protocol::new_secret(),
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
                text("Rust 原生客户端  ·  Windows Host  ·  V0.3 测试版").size(14).color(Color::from_rgb8(156,168,189))].spacing(9),
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
        row![button("生成新密钥").on_press_maybe((!active).then_some(Msg::NewKey)).padding(13),
             horizontal_space(),
             if active {button("停止远程服务").on_press(Msg::Stop).padding(13)}
             else {button("启动远程服务").on_press(Msg::Start).padding(13)}]
            .align_y(iced::Alignment::Center),
    ].spacing(15));

    let safety = card(column![
        heading("02 / 权限与隐私", "只有显式授权之后，远程控制指令才会在电脑上执行"),
        checkbox("允许 Android 端操作鼠标及输入文字", state.allow_input.load(Ordering::SeqCst)).on_toggle(Msg::Allow).size(16),
        text("端到端 AES-256-GCM · 临时配对密钥 · 中转服务器无法解密屏幕画面").size(13).color(Color::from_rgb8(163,178,205)),
        text("仅当前登录桌面。UAC、安全桌面、锁屏及后台服务暂不支持。").size(13).color(Color::from_rgb8(204,178,130)),
    ].spacing(15));
    let status = card(column![
        heading("03 / 运行状态", "实时连接状态 · 自动断线重连（最长等待 30 秒）"),
        text(&state.status).size(18),
        text("Android V0.3：输入相同 WSS 地址、访问口令和会话密钥。").size(13)
            .color(Color::from_rgb8(158,172,199)),
        text("只保存中转服务器地址，不保存访问口令或密钥。停止远控会立即取消重试。").size(13)
            .color(Color::from_rgb8(158,172,199)),
    ].spacing(14));

    container(scrollable(container(column![top, connection, safety, status].spacing(22)).width(Length::Fill).max_width(920).padding(22)))
        .width(Length::Fill).height(Length::Fill)
        .center_x(Length::Fill).into()
}

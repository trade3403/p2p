//! Narrow C ABI for the Flutter Windows UI.
//! All network, cryptographic, DPAPI and input-injection work remains in Rust.
//! Dart receives sanitized snapshots and bounded JPEG frames only.
use crate::{
    host::{self, HostConfig},
    host_pairing::{self, PairingDecision},
    local_settings,
    pairing_api::{self, PairingApiClient},
    pairing_crypto::{self, DeviceIdentity, ViewerEphemeral},
    protocol,
    remote_ops,
    trusted_profile::{self, TrustedProfile},
    viewer::{self, ViewerConfig, ViewerEvent},
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    ffi::{CStr, CString},
    os::raw::c_char,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_EVENTS: usize = 256;
const MAX_FRAME_BYTES: usize = 3 * 1024 * 1024;

#[derive(Clone, serde::Serialize)]
struct Snapshot {
    relay_url: String,
    token_configured: bool,
    vault_error: Option<String>,
    host_running: bool,
    host_status: String,
    device_number: String,
    pairing_code: String,
    pairing_expires_at: i64,
    pending_pairing_request_id: Option<String>,
    pending_viewer_fingerprint: Option<String>,
    allow_input: bool,
    allow_clipboard: bool,
    viewer_connected: bool,
    viewer_status: String,
    viewer_width: u32,
    viewer_height: u32,
    monitors: Value,
    clipboard_text: String,
    pairing_active: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            relay_url: local_settings::load_relay_url(),
            token_configured: false,
            vault_error: None,
            host_running: false,
            host_status: "本机共享未启动".into(),
            device_number: String::new(),
            pairing_code: String::new(),
            pairing_expires_at: 0,
            pending_pairing_request_id: None,
            pending_viewer_fingerprint: None,
            allow_input: false,
            allow_clipboard: false,
            viewer_connected: false,
            viewer_status: "未连接".into(),
            viewer_width: 0,
            viewer_height: 0,
            monitors: json!({"count": 1, "selected": 0}),
            clipboard_text: String::new(),
            pairing_active: false,
        }
    }
}

struct Frame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    version: u64,
}

struct Shared {
    snapshot: Snapshot,
    events: VecDeque<String>,
    frame: Option<Frame>,
    next_frame_version: u64,
}

impl Shared {
    fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            events: VecDeque::new(),
            frame: None,
            next_frame_version: 1,
        }
    }

    fn emit(&mut self, event: Value) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event.to_string());
    }
}

struct HostHandle {
    running: Arc<AtomicBool>,
    allow_input: Arc<AtomicBool>,
    allow_clipboard: Arc<AtomicBool>,
    monitor_index: Arc<AtomicUsize>,
    pairing_decisions: Sender<PairingDecision>,
}

struct ViewerHandle {
    active: Arc<AtomicBool>,
    controls: Sender<String>,
}

struct Engine {
    shared: Arc<Mutex<Shared>>,
    profile: Option<TrustedProfile>,
    relay_url: String,
    relay_token: String,
    vault_error: Option<String>,
    host: Option<HostHandle>,
    viewer: Option<ViewerHandle>,
    pairing_cancel: Option<Arc<AtomicBool>>,
    pairing_active: Option<Arc<AtomicBool>>,
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

fn shared_lock(shared: &Arc<Mutex<Shared>>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn engine() -> &'static Mutex<Engine> {
    ENGINE.get_or_init(|| Mutex::new(Engine::new()))
}

fn engine_lock() -> std::sync::MutexGuard<'static, Engine> {
    engine().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Engine {
    fn new() -> Self {
        crate::tls::initialize();
        let mut snapshot = Snapshot::default();
        let mut profile = None;
        let mut relay_url = snapshot.relay_url.clone();
        let mut relay_token = String::new();
        let mut vault_error = None;

        match trusted_profile::load() {
            Ok(Some(saved)) => {
                relay_url = saved.relay_url.clone();
                relay_token = saved.relay_token.clone();
                snapshot.token_configured = !relay_token.is_empty();
                profile = Some(saved);
            }
            Ok(None) => {}
            Err(error) => {
                // Never replace a DPAPI vault that could not be decrypted.
                vault_error = Some(format!("DPAPI 凭据库无法读取，已拒绝覆盖：{error}"));
                snapshot.vault_error = vault_error.clone();
            }
        }
        snapshot.relay_url = relay_url.clone();
        Self {
            shared: Arc::new(Mutex::new(Shared::new(snapshot))),
            profile,
            relay_url,
            relay_token,
            vault_error,
            host: None,
            viewer: None,
            pairing_cancel: None,
            pairing_active: None,
        }
    }

    fn snapshot(&self) -> Value {
        let shared = shared_lock(&self.shared);
        serde_json::to_value(&shared.snapshot).unwrap_or_else(|_| json!({}))
    }

    fn response(&self, extra: Value) -> Value {
        json!({"ok": true, "snapshot": self.snapshot(), "result": extra})
    }

    fn fail(&self, message: impl Into<String>) -> Value {
        json!({"ok": false, "error": message.into(), "snapshot": self.snapshot()})
    }

    fn command(&mut self, value: Value) -> Value {
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return self.fail("缺少 method");
        };
        match method {
            "snapshot" => self.response(json!({})),
            "configureRelay" => self.configure_relay(&value),
            "startHost" => self.start_host(),
            "stopHost" => self.stop_host(),
            "setRemoteInputAllowed" => self.set_remote_input(value.get("allowed").and_then(Value::as_bool)),
            "setClipboardAllowed" => self.set_clipboard(value.get("allowed").and_then(Value::as_bool)),
            "approvePairingRequest" => self.decide_pairing(&value, true),
            "denyPairingRequest" => self.decide_pairing(&value, false),
            "requestPairing" => self.request_pairing(&value),
            "cancelPairing" => self.cancel_pairing(),
            "connectLegacy" => self.connect_legacy(&value),
            "disconnectViewer" => self.disconnect_viewer(),
            "sendPointer" => self.send_pointer(&value),
            "sendScroll" => self.send_scroll(&value),
            "sendText" => self.send_text(&value),
            "sendKey" => self.send_key(&value),
            "selectMonitor" => self.select_monitor(&value),
            "getClipboardText" => self.get_clipboard(),
            "setClipboardText" => self.set_clipboard_text(&value),
            "setVideoProfile" => self.set_video_profile(&value),
            "revokeLocalTrust" => self.revoke_local_trust(),
            _ => self.fail(format!("不支持的命令：{method}")),
        }
    }

    fn configure_relay(&mut self, value: &Value) -> Value {
        if let Some(error) = self.vault_error.as_ref() {
            return self.fail(error.clone());
        }
        let Some(server) = value.get("server").and_then(Value::as_str).map(str::trim) else {
            return self.fail("中转地址不能为空");
        };
        let Some(token) = value.get("token").and_then(Value::as_str).map(str::trim) else {
            return self.fail("中转访问口令不能为空");
        };
        if !local_settings::is_safe_address(server) {
            return self.fail("中转地址必须是 HTTPS/WSS，且不能包含账号、参数或片段");
        }
        if !(32..=512).contains(&token.len()) {
            return self.fail("中转访问口令长度必须为 32 至 512 字符");
        }

        let session_hex = self.profile.as_ref().map(|p| p.session_hex.clone())
            .unwrap_or_else(|| hex::encode(protocol::new_secret()));
        let profile = TrustedProfile {
            relay_url: server.to_owned(),
            relay_token: token.to_owned(),
            session_hex,
            launch_on_login: self.profile.as_ref().map(|p| p.launch_on_login).unwrap_or(false),
            unattended_input: self.profile.as_ref().map(|p| p.unattended_input).unwrap_or(false),
        };
        if let Err(error) = trusted_profile::save(&profile) {
            return self.fail(format!("无法使用 Windows DPAPI 保存中转凭据：{error}"));
        }
        if let Err(error) = local_settings::save_relay_url(server) {
            return self.fail(format!("凭据已加密保存，但公开服务器地址保存失败：{error}"));
        }
        self.relay_url = server.to_owned();
        self.relay_token = token.to_owned();
        self.profile = Some(profile);
        {
            let mut shared = shared_lock(&self.shared);
            shared.snapshot.relay_url = self.relay_url.clone();
            shared.snapshot.token_configured = true;
            shared.snapshot.vault_error = None;
            shared.emit(json!({"type":"configuration","message":"中转地址与访问口令已通过 Windows DPAPI 保存"}));
        }
        self.response(json!({"saved": true}))
    }

    fn start_host(&mut self) -> Value {
        if let Some(host) = self.host.as_ref() {
            if host.running.load(Ordering::SeqCst) {
                return self.response(json!({"already_running": true}));
            }
        }
        if self.vault_error.is_some() {
            return self.fail(self.vault_error.clone().unwrap_or_default());
        }
        let Some(profile) = self.profile.clone() else {
            return self.fail("请先在网络设置中配置中转地址和访问口令");
        };
        if profile.relay_token.len() < 32 {
            return self.fail("中转访问口令未配置");
        }

        let identity_parts = match trusted_profile::load_or_create_device_identity() {
            Ok(value) => value,
            Err(error) => return self.fail(format!("无法安全读取/创建本机设备身份：{error}")),
        };
        let device_id = match hex::decode(identity_parts.0) {
            Ok(bytes) => match <[u8; 16]>::try_from(bytes) {
                Ok(id) => id,
                Err(_) => return self.fail("本机设备身份长度无效"),
            },
            Err(_) => return self.fail("本机设备身份格式无效"),
        };
        let identity = DeviceIdentity::from_parts(device_id, identity_parts.1);
        let secret_bytes = match hex::decode(&profile.session_hex)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).map_err(|_| hex::FromHexError::InvalidStringLength))
        {
            Ok(secret) => secret,
            Err(_) => return self.fail("DPAPI 会话密钥格式无效"),
        };

        let running = Arc::new(AtomicBool::new(true));
        let allow_input = Arc::new(AtomicBool::new(false));
        let allow_clipboard = Arc::new(AtomicBool::new(false));
        let monitor_index = Arc::new(AtomicUsize::new(0));
        let (messages, message_rx) = mpsc::channel::<String>();
        let (decision_tx, decision_rx) = mpsc::channel::<PairingDecision>();
        let config = HostConfig {
            server: profile.relay_url.clone(),
            relay_token: profile.relay_token.clone(),
            secret: secret_bytes,
            running: running.clone(),
            allow_input: allow_input.clone(),
            allow_clipboard: allow_clipboard.clone(),
            monitor_index: monitor_index.clone(),
            lan_direct: Arc::new(AtomicBool::new(false)),
            lan_active: Arc::new(AtomicBool::new(false)),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(12)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            messages,
        };
        host_pairing::start(config, identity, decision_rx);

        let shared = self.shared.clone();
        thread::spawn(move || {
            while let Ok(message) = message_rx.recv() {
                let mut state = shared_lock(&shared);
                if let Ok(event) = serde_json::from_str::<Value>(&message) {
                    match event.get("event").and_then(Value::as_str).unwrap_or("") {
                        "pairing_device" => {
                            state.snapshot.device_number = event["device_number"].as_str().unwrap_or("").to_owned();
                        }
                        "pairing_code" => {
                            state.snapshot.device_number = event["device_number"].as_str().unwrap_or("").to_owned();
                            state.snapshot.pairing_code = event["code"].as_str().unwrap_or("").to_owned();
                            state.snapshot.pairing_expires_at = event["expires_at"].as_i64().unwrap_or(0);
                        }
                        "pairing_request" => {
                            state.snapshot.pending_pairing_request_id = event["request_id"].as_str().map(str::to_owned);
                            state.snapshot.pending_viewer_fingerprint = event["viewer_fingerprint"].as_str().map(str::to_owned);
                        }
                        "pairing_request_expired" | "pairing_request_denied" | "pairing_approved" => {
                            state.snapshot.pending_pairing_request_id = None;
                            state.snapshot.pending_viewer_fingerprint = None;
                        }
                        "pairing_stopped" => {
                            state.snapshot.pairing_code.clear();
                            state.snapshot.pairing_expires_at = 0;
                        }
                        _ => {}
                    }
                    state.snapshot.host_status = event.get("message").and_then(Value::as_str).unwrap_or(&message).to_owned();
                    state.emit(event);
                } else {
                    state.snapshot.host_status = message.clone();
                    state.emit(json!({"type":"hostStatus","message":message}));
                }
            }
        });

        self.host = Some(HostHandle {
            running,
            allow_input,
            allow_clipboard,
            monitor_index,
            pairing_decisions: decision_tx,
        });
        {
            let mut shared = shared_lock(&self.shared);
            shared.snapshot.host_running = true;
            shared.snapshot.host_status = "正在向中转服务器注册本机设备".into();
            shared.snapshot.allow_input = false;
            shared.snapshot.allow_clipboard = false;
            shared.emit(json!({"type":"hostState","running":true}));
        }
        self.response(json!({"started": true}))
    }

    fn stop_host(&mut self) -> Value {
        if let Some(host) = self.host.take() {
            host.allow_input.store(false, Ordering::SeqCst);
            host.allow_clipboard.store(false, Ordering::SeqCst);
            host.running.store(false, Ordering::SeqCst);
        }
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.host_running = false;
        shared.snapshot.allow_input = false;
        shared.snapshot.allow_clipboard = false;
        shared.snapshot.pairing_code.clear();
        shared.snapshot.pairing_expires_at = 0;
        shared.snapshot.pending_pairing_request_id = None;
        shared.snapshot.pending_viewer_fingerprint = None;
        shared.snapshot.host_status = "本机共享已停止".into();
        shared.emit(json!({"type":"hostState","running":false}));
        drop(shared);
        self.response(json!({"stopped": true}))
    }

    fn set_remote_input(&mut self, allowed: Option<bool>) -> Value {
        let Some(allowed) = allowed else { return self.fail("allowed 必须是布尔值"); };
        let Some(host) = self.host.as_ref() else { return self.fail("请先启动本机共享"); };
        host.allow_input.store(allowed, Ordering::SeqCst);
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.allow_input = allowed;
        shared.emit(json!({"type":"permission","name":"remoteInput","allowed":allowed}));
        drop(shared);
        self.response(json!({"allowed": allowed}))
    }

    fn set_clipboard(&mut self, allowed: Option<bool>) -> Value {
        let Some(allowed) = allowed else { return self.fail("allowed 必须是布尔值"); };
        let Some(host) = self.host.as_ref() else { return self.fail("请先启动本机共享"); };
        host.allow_clipboard.store(allowed, Ordering::SeqCst);
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.allow_clipboard = allowed;
        shared.emit(json!({"type":"permission","name":"clipboard","allowed":allowed}));
        drop(shared);
        self.response(json!({"allowed": allowed}))
    }

    fn decide_pairing(&mut self, value: &Value, approved: bool) -> Value {
        let Some(request_id) = value.get("requestId").and_then(Value::as_str) else {
            return self.fail("缺少 requestId");
        };
        let current = shared_lock(&self.shared).snapshot.pending_pairing_request_id.clone();
        if current.as_deref() != Some(request_id) {
            return self.fail("配对请求已过期或不再有效");
        }
        let Some(host) = self.host.as_ref() else { return self.fail("本机共享未运行"); };
        if host.pairing_decisions.send(PairingDecision::Respond {
            request_id: request_id.to_owned(), approved,
        }).is_err() {
            return self.fail("数字配对服务已停止，请重新启动共享");
        }
        self.response(json!({"approved": approved}))
    }

    fn request_pairing(&mut self, value: &Value) -> Value {
        if self.viewer.as_ref().is_some_and(|v| v.active.load(Ordering::SeqCst)) {
            return self.fail("请先断开当前远程会话");
        }
        if self.pairing_active.as_ref().is_some_and(|v| v.load(Ordering::SeqCst)) {
            return self.fail("正在进行配对，请等待或取消当前请求");
        }
        let Some(number) = value.get("deviceNumber").and_then(Value::as_str) else {
            return self.fail("缺少设备号");
        };
        let Some(code) = value.get("pairingCode").and_then(Value::as_str) else {
            return self.fail("缺少动态验证码");
        };
        if number.len() != 9 || !number.bytes().all(|b| b.is_ascii_digit()) {
            return self.fail("设备号必须是9位数字");
        }
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return self.fail("动态验证码必须是6位数字");
        }
        if self.relay_token.len() < 32 {
            return self.fail("请先配置中转服务器访问口令");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(true));
        self.pairing_cancel = Some(cancel.clone());
        self.pairing_active = Some(active.clone());
        {
            let mut shared = shared_lock(&self.shared);
            shared.snapshot.pairing_active = true;
            shared.snapshot.viewer_status = "正在验证目标设备身份".into();
            shared.emit(json!({"type":"pairingState","active":true}));
        }
        let server = self.relay_url.clone();
        let token = self.relay_token.clone();
        let shared = self.shared.clone();
        let device_number = number.to_owned();
        let pairing_code = code.to_owned();
        thread::spawn(move || {
            let result = pair_and_derive(
                &server, &token, &device_number, &pairing_code, &cancel, &shared
            );
            active.store(false, Ordering::SeqCst);
            {
                let mut state = shared_lock(&shared);
                state.snapshot.pairing_active = false;
                state.emit(json!({"type":"pairingState","active":false}));
            }
            match result {
                Ok(mut secret) => {
                    if cancel.load(Ordering::SeqCst) {
                        secret.fill(0);
                        return;
                    }
                    let mut engine = engine_lock();
                    engine.start_viewer_with_secret(secret);
                    secret.fill(0);
                }
                Err(error) => {
                    let mut state = shared_lock(&shared);
                    state.snapshot.viewer_status = error.clone();
                    state.emit(json!({"type":"pairingError","message":error}));
                }
            }
        });
        self.response(json!({"pairing_started": true}))
    }

    fn cancel_pairing(&mut self) -> Value {
        if let Some(cancel) = self.pairing_cancel.as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.response(json!({"cancelled": true}))
    }

    fn connect_legacy(&mut self, value: &Value) -> Value {
        let Some(secret_hex) = value.get("sessionKey").and_then(Value::as_str) else {
            return self.fail("缺少64位会话密钥");
        };
        let bytes = match hex::decode(secret_hex.trim()) {
            Ok(bytes) => bytes,
            Err(_) => return self.fail("会话密钥必须为64位十六进制"),
        };
        let secret = match <[u8; 32]>::try_from(bytes) {
            Ok(secret) => secret,
            Err(_) => return self.fail("会话密钥必须是32字节"),
        };
        if self.relay_token.len() < 32 {
            return self.fail("请先配置中转服务器访问口令");
        }
        self.start_viewer_with_secret(secret);
        self.response(json!({"connected": true}))
    }

    fn start_viewer_with_secret(&mut self, secret: [u8; 32]) {
        if let Some(old) = self.viewer.take() {
            old.active.store(false, Ordering::SeqCst);
        }
        let (control_tx, controls) = mpsc::channel::<String>();
        let (events, receiver) = mpsc::channel::<ViewerEvent>();
        let active = Arc::new(AtomicBool::new(true));
        viewer::start(ViewerConfig {
            server: self.relay_url.clone(),
            token: self.relay_token.clone(),
            secret,
            active: active.clone(),
            events,
            controls,
        });
        let shared = self.shared.clone();
        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                let mut state = shared_lock(&shared);
                match event {
                    ViewerEvent::State(message) => {
                        state.snapshot.viewer_status = message.clone();
                        state.emit(json!({"type":"viewerState","message":message}));
                    }
                    ViewerEvent::Connected => {
                        state.snapshot.viewer_connected = true;
                        state.snapshot.viewer_status = "WSS 已连接，等待远程桌面画面".into();
                        state.emit(json!({"type":"viewerConnected"}));
                    }
                    ViewerEvent::Disconnected => {
                        state.snapshot.viewer_connected = false;
                        state.snapshot.viewer_width = 0;
                        state.snapshot.viewer_height = 0;
                        state.frame = None;
                        state.snapshot.viewer_status = "远程连接已断开".into();
                        state.emit(json!({"type":"viewerDisconnected"}));
                    }
                    ViewerEvent::Jpeg(bytes, width, height) => {
                        if bytes.len() <= MAX_FRAME_BYTES && width > 0 && height > 0
                            && width <= 8192 && height <= 8192 {
                            let version = state.next_frame_version;
                            state.next_frame_version = state.next_frame_version.wrapping_add(1).max(1);
                            state.snapshot.viewer_width = width;
                            state.snapshot.viewer_height = height;
                            state.frame = Some(Frame { bytes, width, height, version });
                        }
                    }
                    ViewerEvent::Monitors(value) => {
                        state.snapshot.monitors = serde_json::from_str(&value).unwrap_or(json!({"count":1,"selected":0}));
                        state.emit(json!({"type":"monitors","value":state.snapshot.monitors}));
                    }
                    ViewerEvent::Clipboard(value) => {
                        state.snapshot.clipboard_text = value.clone();
                        state.emit(json!({"type":"clipboard","text":value}));
                    }
                }
            }
        });
        self.viewer = Some(ViewerHandle { active, controls: control_tx });
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.viewer_connected = false;
        shared.snapshot.viewer_status = "正在连接远程 Windows".into();
        shared.snapshot.viewer_width = 0;
        shared.snapshot.viewer_height = 0;
        shared.frame = None;
        shared.emit(json!({"type":"viewerState","message":"正在连接远程 Windows"}));
    }

    fn disconnect_viewer(&mut self) -> Value {
        if let Some(viewer) = self.viewer.take() {
            let _ = viewer.controls.send(json!({"type":"up","x":0.5,"y":0.5}).to_string());
            viewer.active.store(false, Ordering::SeqCst);
        }
        if let Some(cancel) = self.pairing_cancel.as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.viewer_connected = false;
        shared.snapshot.viewer_width = 0;
        shared.snapshot.viewer_height = 0;
        shared.snapshot.viewer_status = "已手动断开".into();
        shared.frame = None;
        shared.emit(json!({"type":"viewerDisconnected"}));
        drop(shared);
        self.response(json!({"disconnected": true}))
    }

    fn send_control(&self, value: Value) -> Result<(), String> {
        let Some(viewer) = self.viewer.as_ref() else { return Err("尚未连接远程电脑".into()); };
        if !viewer.active.load(Ordering::SeqCst) { return Err("远程会话已断开".into()); }
        let text = value.to_string();
        if text.len() > 8192 { return Err("控制命令过长".into()); }
        viewer.controls.send(text).map_err(|_| "远程控制通道已关闭".into())
    }

    fn send_pointer(&mut self, value: &Value) -> Value {
        let Some(kind) = value.get("kind").and_then(Value::as_str) else { return self.fail("缺少鼠标事件类型"); };
        if !matches!(kind, "move" | "down" | "up" | "right_click") {
            return self.fail("不支持的鼠标事件");
        }
        let x = value.get("x").and_then(Value::as_f64).unwrap_or(0.5);
        let y = value.get("y").and_then(Value::as_f64).unwrap_or(0.5);
        if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return self.fail("鼠标坐标必须是0至1之间的归一化坐标");
        }
        match self.send_control(json!({"type":kind,"x":x,"y":y})) {
            Ok(()) => self.response(json!({"sent":true})),
            Err(error) => self.fail(error),
        }
    }

    fn send_scroll(&mut self, value: &Value) -> Value {
        let delta = value.get("delta").and_then(Value::as_i64).unwrap_or(0);
        if delta == 0 || !(-10..=10).contains(&delta) { return self.fail("滚动幅度必须在-10至10之间"); }
        match self.send_control(json!({"type":"scroll","delta":delta})) {
            Ok(()) => self.response(json!({"sent":true})),
            Err(error) => self.fail(error),
        }
    }

    fn send_text(&mut self, value: &Value) -> Value {
        let Some(text) = value.get("text").and_then(Value::as_str) else { return self.fail("缺少文字内容"); };
        if text.is_empty() || text.chars().count() > 250 { return self.fail("文字长度必须为1至250个字符"); }
        match self.send_control(json!({"type":"text","text":text})) {
            Ok(()) => self.response(json!({"sent":true})),
            Err(error) => self.fail(error),
        }
    }

    fn send_key(&mut self, value: &Value) -> Value {
        let Some(key) = value.get("key").and_then(Value::as_str) else { return self.fail("缺少按键名称"); };
        if !remote_ops::valid_remote_key(key) { return self.fail("按键不在远程输入白名单中"); }
        match self.send_control(json!({"type":"key","key":key})) {
            Ok(()) => self.response(json!({"sent":true})),
            Err(error) => self.fail(error),
        }
    }

    fn select_monitor(&mut self, value: &Value) -> Value {
        let Some(index) = value.get("index").and_then(Value::as_u64) else { return self.fail("显示器索引无效"); };
        let count = shared_lock(&self.shared).snapshot.monitors.get("count").and_then(Value::as_u64).unwrap_or(1);
        if index >= count || index >= 16 { return self.fail("显示器索引超出范围"); }
        match self.send_control(json!({"type":"monitor","index":index})) {
            Ok(()) => self.response(json!({"selected":index})),
            Err(error) => self.fail(error),
        }
    }

    fn get_clipboard(&mut self) -> Value {
        match self.send_control(json!({"type":"clipboard_get"})) {
            Ok(()) => self.response(json!({"requested":true})),
            Err(error) => self.fail(error),
        }
    }

    fn set_clipboard_text(&mut self, value: &Value) -> Value {
        let Some(text) = value.get("text").and_then(Value::as_str) else { return self.fail("缺少剪贴板文字"); };
        if !remote_ops::valid_clipboard_text(text) { return self.fail("剪贴板文字为空或超过4096字节"); }
        match self.send_control(json!({"type":"clipboard_set","text":text})) {
            Ok(()) => self.response(json!({"sent":true})),
            Err(error) => self.fail(error),
        }
    }

    fn set_video_profile(&mut self, value: &Value) -> Value {
        let Some(host) = self.host.as_ref() else { return self.fail("请先启动本机共享"); };
        let fps = value.get("fps").and_then(Value::as_u64).unwrap_or(12);
        if !matches!(fps, 6 | 12 | 20) { return self.fail("帧率只支持6、12或20 FPS"); }
        let h264 = value.get("h264").and_then(Value::as_bool).unwrap_or(false);
        // The existing Windows Viewer requests JPEG on connect. Keep JPEG as
        // the interoperable default until codec negotiation is exposed in Flutter.
        let _ = h264;
        let _ = fps;
        let _ = host;
        self.response(json!({"applied":false,"reason":"当前桥接阶段保持现有 JPEG 兼容默认值"}))
    }

    fn revoke_local_trust(&mut self) -> Value {
        if self.host.as_ref().is_some_and(|host| host.running.load(Ordering::SeqCst)) {
            return self.fail("请先停止本机共享，再撤销本机可信凭据");
        }
        if let Some(viewer) = self.viewer.take() {
            viewer.active.store(false, Ordering::SeqCst);
        }
        if let Err(error) = trusted_profile::revoke() {
            return self.fail(format!("撤销 DPAPI 凭据失败：{error}"));
        }
        let _ = trusted_profile::set_login_start(false);
        self.profile = None;
        self.relay_token.clear();
        self.vault_error = None;
        let mut shared = shared_lock(&self.shared);
        shared.snapshot.token_configured = false;
        shared.snapshot.vault_error = None;
        shared.snapshot.viewer_connected = false;
        shared.snapshot.viewer_status = "本机凭据已撤销".into();
        shared.emit(json!({"type":"trustRevoked"}));
        drop(shared);
        self.response(json!({"revoked":true}))
    }
}

fn emit_shared(shared: &Arc<Mutex<Shared>>, event: Value) {
    shared_lock(shared).emit(event);
}

fn pair_and_derive(
    server: &str,
    token: &str,
    device_number: &str,
    code: &str,
    cancelled: &AtomicBool,
    shared: &Arc<Mutex<Shared>>,
) -> Result<[u8; 32], String> {
    let result = (|| -> anyhow::Result<[u8; 32]> {
        use rand::RngCore;
        let api = PairingApiClient::new(server, token)?;
        let device = api.lookup_device(device_number)?;
        anyhow::ensure!(device.online, "目标电脑当前不在线");
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        anyhow::ensure!(device.pairing_expires_at > now, "目标电脑动态验证码已过期");
        let identity_public = pairing_api::decode_base64url(&device.identity_public_key, 32)?;
        let identity_public = <[u8; 32]>::try_from(identity_public)
            .map_err(|_| anyhow::anyhow!("invalid identity public key length"))?;
        let pairing_public = pairing_api::decode_base64url(&device.pairing_public_key, 32)?;
        let pairing_public = <[u8; 32]>::try_from(pairing_public)
            .map_err(|_| anyhow::anyhow!("invalid pairing public key length"))?;
        anyhow::ensure!(
            pairing_crypto::verify_pairing_window(
                identity_public, device_number, device.pairing_expires_at,
                &device.pairing_public_key, &device.pairing_signature
            ),
            "目标设备身份签名验证失败"
        );
        anyhow::ensure!(!cancelled.load(Ordering::SeqCst), "配对已取消");
        let viewer = ViewerEphemeral::generate();
        let mut nonce = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let proof = viewer.proof(device_number, pairing_public, nonce, code)?;
        let request = api.request_pairing(device_number, &viewer.public_key, &nonce, &proof)?;
        emit_shared(shared, json!({"type":"pairingStatus","message":"验证码已提交，等待被控电脑用户确认"}));
        while SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() < request.expires_at.max(0) as u64 {
            if cancelled.load(Ordering::SeqCst) { anyhow::bail!("配对已取消"); }
            match api.request_status(&request.request_id)?.status.as_str() {
                "approved" => return viewer.session_secret(device_number, pairing_public, nonce),
                "denied" => anyhow::bail!("验证码错误、已过期，或被控电脑拒绝了本次连接"),
                "expired" => anyhow::bail!("配对请求已过期，请重新发起"),
                "pending" => {}
                other => anyhow::bail!("未知配对状态：{other}"),
            }
            thread::sleep(Duration::from_millis(900));
        }
        anyhow::bail!("等待被控电脑确认超时")
    })();
    result.map_err(|error| error.to_string())
}

fn c_string(value: String) -> *mut c_char {
    CString::new(value.replace('\0', "�"))
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

fn command_inner(input: *const c_char) -> *mut c_char {
    if input.is_null() { return c_string(r#"{"ok":false,"error":"null command"}"#.into()); }
    let bytes = unsafe { CStr::from_ptr(input) }.to_bytes();
    if bytes.len() > MAX_COMMAND_BYTES {
        return c_string(r#"{"ok":false,"error":"command too large"}"#.into());
    }
    let parsed = match std::str::from_utf8(bytes).ok().and_then(|s| serde_json::from_str::<Value>(s).ok()) {
        Some(value) => value,
        None => return c_string(r#"{"ok":false,"error":"invalid JSON command"}"#.into()),
    };
    let mut engine = engine_lock();
    c_string(engine.command(parsed).to_string())
}

#[no_mangle]
pub extern "C" fn p2p_bridge_init() -> i32 {
    let _ = engine();
    1
}

#[no_mangle]
pub extern "C" fn p2p_bridge_command_json(input: *const c_char) -> *mut c_char {
    std::panic::catch_unwind(|| command_inner(input))
        .unwrap_or_else(|_| c_string(r#"{"ok":false,"error":"native bridge panic"}"#.into()))
}

#[no_mangle]
pub extern "C" fn p2p_bridge_poll_event() -> *mut c_char {
    let event = {
        let engine = engine_lock();
        let mut shared = shared_lock(&engine.shared);
        shared.events.pop_front()
    };
    event.map(c_string).unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn p2p_bridge_latest_frame_version() -> u64 {
    let engine = engine_lock();
    shared_lock(&engine.shared).frame.as_ref().map(|frame| frame.version).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn p2p_bridge_latest_frame_size() -> usize {
    let engine = engine_lock();
    shared_lock(&engine.shared).frame.as_ref().map(|frame| frame.bytes.len()).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn p2p_bridge_copy_latest_frame(
    output: *mut u8,
    capacity: usize,
    width: *mut u32,
    height: *mut u32,
    version: *mut u64,
) -> usize {
    if output.is_null() || width.is_null() || height.is_null() || version.is_null() {
        return 0;
    }
    let engine = engine_lock();
    let shared = shared_lock(&engine.shared);
    let Some(frame) = shared.frame.as_ref() else { return 0; };
    if frame.bytes.len() > capacity { return 0; }
    unsafe {
        ptr::copy_nonoverlapping(frame.bytes.as_ptr(), output, frame.bytes.len());
        *width = frame.width;
        *height = frame.height;
        *version = frame.version;
    }
    frame.bytes.len()
}

#[no_mangle]
pub extern "C" fn p2p_bridge_free_string(value: *mut c_char) {
    if !value.is_null() {
        unsafe { drop(CString::from_raw(value)); }
    }
}

#[no_mangle]
pub extern "C" fn p2p_bridge_shutdown() {
    let mut engine = engine_lock();
    if let Some(host) = engine.host.take() {
        host.allow_input.store(false, Ordering::SeqCst);
        host.allow_clipboard.store(false, Ordering::SeqCst);
        host.running.store(false, Ordering::SeqCst);
    }
    if let Some(viewer) = engine.viewer.take() {
        let _ = viewer.controls.send(json!({"type":"up","x":0.5,"y":0.5}).to_string());
        viewer.active.store(false, Ordering::SeqCst);
    }
    if let Some(cancel) = engine.pairing_cancel.as_ref() {
        cancel.store(true, Ordering::SeqCst);
    }
}

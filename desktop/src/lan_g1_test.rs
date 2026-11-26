//! Windows Runner loopback integration: real production host::run, JPEG capture,
//! WebSocket sends and authenticated LAN v4 session, not a physical LAN or VPS.
use super::*;
use std::sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize}, Arc, mpsc};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{timeout, timeout_at, Instant};
use tokio_tungstenite::connect_async;

#[derive(Debug, serde::Serialize)]
struct Snapshot {
    active: bool,
    relay_bytes: u64,
    relay_frames: u64,
    lan_bytes: u64,
    lan_frames: u64,
    suppressed: u64,
}
fn snapshot(c: &HostConfig) -> Snapshot {
    let s = &c.transport_video;
    Snapshot {
        active: c.lan_active.load(Ordering::SeqCst),
        relay_bytes: s.relay_bytes.load(Ordering::Relaxed),
        relay_frames: s.relay_frames.load(Ordering::Relaxed),
        lan_bytes: s.lan_bytes.load(Ordering::Relaxed),
        lan_frames: s.lan_frames.load(Ordering::Relaxed),
        suppressed: s.relay_suppressed_frames.load(Ordering::Relaxed),
    }
}

#[derive(Default, Debug, serde::Serialize)]
struct Observed {
    bytes: u64,
    frames: u64,
    pong: bool,
    ws_pings: u64,
}
struct Peer<S> {
    socket: WebSocketStream<S>,
    key: [u8;32],
    sender: Option<protocol::SequenceSender>,
    guard: Option<protocol::ReplayGuard>,
}
impl<S: AsyncRead + AsyncWrite + Unpin> Peer<S> {
    fn new(socket: WebSocketStream<S>, key: [u8;32], lan: bool) -> Self {
        Self { socket, key,
            sender: lan.then(|| protocol::SequenceSender::new(protocol::VIEW_DIRECTION)),
            guard: lan.then(|| protocol::ReplayGuard::new(protocol::HOST_DIRECTION)) }
    }
    async fn send(&mut self, kind: u8, payload: &[u8]) {
        let packet = match &mut self.sender {
            Some(sender) => sender.encrypt(&self.key, kind, payload),
            None => protocol::encrypt(&self.key, kind, payload),
        }.unwrap();
        timeout(Duration::from_secs(5), self.socket.send(Message::Binary(packet)))
            .await.expect("peer send timeout").expect("peer send failed");
    }
    // Keep transport traffic moving while polling a real OS side effect.
    async fn service_until<F: std::future::Future>(&mut self, work: F) -> F::Output {
        tokio::pin!(work);
        loop {
            tokio::select! {
                biased;
                done = &mut work => return done,
                next = self.socket.next() => {
                    match next {
                        Some(Ok(Message::Binary(packet))) => {
                            protocol::decrypt(&self.key, &packet).expect("host packet authentication failed");
                            if let Some(guard) = &mut self.guard {
                                assert!(guard.accept(&packet), "host LAN replay/direction violation");
                            }
                        }
                        Some(Ok(Message::Ping(_))) => {
                            timeout(Duration::from_secs(5), self.socket.flush()).await.unwrap().unwrap();
                        }
                        Some(Ok(Message::Close(_))) | None => panic!("unexpected peer close while checking OS input"),
                        Some(Err(error)) => panic!("peer read failed: {}", error),
                        _ => {}
                    }
                }
            }
        }
    }
    async fn sample(&mut self, seconds: u64, expected_ping: u64) -> Observed {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut result = Observed::default();
        loop {
            let message = match timeout_at(deadline, self.socket.next()).await {
                Err(_) => break,
                Ok(Some(Ok(message))) => message,
                _ => panic!("peer disconnected during observation"),
            };
            match message {
                Message::Binary(packet) => {
                    let (kind, payload) = protocol::decrypt(&self.key, &packet)
                        .expect("host packet authentication failed");
                    if let Some(guard) = &mut self.guard {
                        assert!(guard.accept(&packet), "host LAN replay/direction violation");
                    }
                    match kind {
                        protocol::FRAME => {
                            // Verify a real decodable screen JPEG, never log its contents.
                            if result.frames == 0 {
                                let image = image::load_from_memory_with_format(
                                    &payload, image::ImageFormat::Jpeg).expect("invalid screen JPEG");
                                assert!(image.width() >= 2 && image.height() >= 2);
                            }
                            result.frames += 1;
                            result.bytes += packet.len() as u64;
                        }
                        protocol::AVC => panic!("unexpected AVC in JPEG-only G1 test"),
                        protocol::PONG if payload.as_slice() == &expected_ping.to_be_bytes()[..] => {
                            result.pong = true;
                        }
                        _ => {}
                    }
                }
                Message::Ping(_) => {
                    result.ws_pings += 1;
                    timeout(Duration::from_secs(5), self.socket.flush()).await.unwrap().unwrap();
                }
                Message::Close(_) => panic!("host closed during observation"),
                _ => {}
            }
            if Instant::now() >= deadline { break; }
        }
        result
    }
}
async fn binary<S: AsyncRead + AsyncWrite + Unpin>(socket: &mut WebSocketStream<S>) -> Vec<u8> {
    match timeout(Duration::from_secs(8), socket.next()).await {
        Ok(Some(Ok(Message::Binary(packet)))) => packet,
        _ => panic!("expected encrypted LAN handshake message"),
    }
}
fn evidence(stage: &str, data: serde_json::Value) {
    use std::io::Write;
    let line = serde_json::json!({"stage":stage,"scope":"windows_runner_loopback_ws_jpeg","data":data}).to_string();
    eprintln!("{}", line);
    if let Ok(path) = std::env::var("P2P_G1_REPORT") {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)
            .expect("open G1 evidence");
        writeln!(file, "{}", line).expect("write G1 evidence");
    }
}

#[tokio::test]
#[ignore = "requires interactive Windows desktop; mandatory dedicated V0.21 Windows CI step"]
async fn windows_real_host_lan_suppresses_relay_video() {
    // The production host runs in a current-thread runtime. Local tasks avoid
    // imposing Send on its platform-specific native input/clipboard objects.
    tokio::task::LocalSet::new().run_until(async {
        let relay_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let relay_address = relay_listener.local_addr().unwrap();
        let lan_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let lan_address = lan_listener.local_addr().unwrap();
        let secret = protocol::new_secret();
        let (messages, logs) = mpsc::channel();
        let config = HostConfig {
            server: format!("ws://{relay_address}"),
            relay_token: "local-test-only".into(), secret,
            running: Arc::new(AtomicBool::new(true)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(true)),
            lan_active: Arc::new(AtomicBool::new(false)),
            control_gate: Arc::new(host::ControlGate::default()),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        evidence("start", serde_json::json!({"host":snapshot(&config)}));
        let relay_config = config.clone();
        let relay_task = tokio::task::spawn_local(async move { host::run(&relay_config).await });
        let (stream, _) = timeout(Duration::from_secs(15), relay_listener.accept())
            .await.expect("host relay connect timeout").unwrap();
        let socket = timeout(Duration::from_secs(10), accept_async(stream)).await.unwrap().unwrap();
        let mut relay = Peer::new(socket, secret, false);
        let before = snapshot(&config);
        relay.send(protocol::PING, &1u64.to_be_bytes()).await;
        let baseline = relay.sample(10, 1).await;
        let after = snapshot(&config);
        evidence("relay_baseline", serde_json::json!({"before":before,"after":after,"relay":baseline}));
        assert!(!after.active && after.lan_frames == 0 && after.lan_bytes == 0);
        assert!(after.relay_bytes > before.relay_bytes && after.relay_frames >= before.relay_frames + 2);
        assert!(baseline.frames >= 2 && baseline.bytes > 0 && baseline.pong);

        // Use the exact production authorization and activation functions,
        // never assign lan_active or increment host video counters in the test.
        let lan_config = config.clone();
        let lan_task = tokio::task::spawn_local(async move {
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip()).await;
        });
        let (mut socket, _) = timeout(Duration::from_secs(10),
            connect_async(format!("ws://{lan_address}/direct"))).await.unwrap().unwrap();
        let client = protocol::new_secret();
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(HELLO_PAYLOAD, &client, &[])).unwrap())).await.unwrap();
        let (kind, challenge) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        let prefix = transcript(CHALLENGE_PAYLOAD, &client, &[]);
        let server = challenge.strip_prefix(prefix.as_slice()).unwrap();
        assert_eq!(server.len(), NONCE_LEN);
        assert!(!config.lan_active.load(Ordering::SeqCst), "LAN active before proof");
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(PROOF_PAYLOAD, &client, server)).unwrap())).await.unwrap();
        let (kind, ready) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        assert_eq!(ready, transcript(READY_PAYLOAD, &client, server));
        let mut lan = Peer::new(socket, business_key(&secret, &client, server), true);
        // Drain any already-in-flight relay video, without counting it as
        // steady-state leakage; both sockets remain continuously serviced.
        let (drain_relay, drain_lan) = tokio::join!(relay.sample(4, 99), lan.sample(4, 99));
        let stable_before = snapshot(&config);
        evidence("lan_settle", serde_json::json!({"host":stable_before,"relay":drain_relay,"lan":drain_lan}));
        assert!(stable_before.active);
        // Both peers send authenticated requests. Only LAN may reach the
        // production JPEG codec-control handler / application PING during LAN.
        let _ = logs.try_iter().count();
        relay.send(protocol::CONTROL, br#"{"type":"video_codec","codec":"jpeg"}"#).await;
        relay.send(protocol::PING, &2u64.to_be_bytes()).await;
        lan.send(protocol::CONTROL, br#"{"type":"video_codec","codec":"jpeg"}"#).await;
        lan.send(protocol::PING, &3u64.to_be_bytes()).await;
        let (blocked, direct) = tokio::join!(relay.sample(10, 2), lan.sample(10, 3));
        let stable_after = snapshot(&config);
        let codec_controls = logs.try_iter().filter(|s| s.contains("已接收 Viewer 兼容请求")).count();
        evidence("lan_stable", serde_json::json!({"before":stable_before,"after":stable_after,
            "relay":blocked,"lan":direct,"accepted_jpeg_codec_controls":codec_controls}));
        assert!(stable_after.active);
        assert_eq!(stable_after.relay_bytes, stable_before.relay_bytes);
        assert_eq!(stable_after.relay_frames, stable_before.relay_frames);
        assert!(stable_after.lan_bytes > stable_before.lan_bytes);
        assert!(stable_after.lan_frames >= stable_before.lan_frames + 2);
        assert!(stable_after.suppressed > stable_before.suppressed);
        assert_eq!((blocked.frames, blocked.bytes), (0, 0));
        assert!(!blocked.pong && direct.pong && codec_controls == 1);
        assert!(direct.frames >= 2 && direct.bytes > 0 && direct.ws_pings > 0);

        // Actual Windows clipboard side effect, tested separately from the
        // ten-second video sample. Do not read or log pre-existing clipboard data.
        config.control_gate.set_clipboard(&config.allow_clipboard, true);
        {
            let mut clipboard = arboard::Clipboard::new().expect("test clipboard unavailable");
            clipboard.set_text("G2b-local-marker").expect("initialize test clipboard");
        }
        relay.send(protocol::CONTROL, br#"{"type":"clipboard_set","text":"G2b-blocked-relay"}"#).await;
        let _ = tokio::join!(relay.sample(3, 99), lan.sample(3, 99));
        {
            let mut clipboard = arboard::Clipboard::new().unwrap();
            assert!(clipboard.get_text().unwrap() == "G2b-local-marker", "inactive relay mutated clipboard");
        }
        lan.send(protocol::CONTROL, br#"{"type":"clipboard_set","text":"G2b-authorized-lan"}"#).await;
        let _ = tokio::join!(relay.sample(3, 99), lan.sample(3, 99));
        {
            let mut clipboard = arboard::Clipboard::new().unwrap();
            assert!(clipboard.get_text().unwrap() == "G2b-authorized-lan", "LAN clipboard command not applied");
        }
        config.control_gate.set_clipboard(&config.allow_clipboard, false);
        evidence("clipboard_ownership", serde_json::json!({"relay_blocked":true,"lan_applied":true}));

        timeout(Duration::from_secs(5), lan.socket.close(None)).await.unwrap().unwrap();
        drop(lan);
        timeout(Duration::from_secs(12), lan_task).await.expect("LAN cleanup timeout").unwrap();
        let resume_before = snapshot(&config);
        assert!(!resume_before.active);
        relay.send(protocol::PING, &4u64.to_be_bytes()).await;
        let resumed = relay.sample(10, 4).await;
        let resume_after = snapshot(&config);
        evidence("relay_resumed", serde_json::json!({"before":resume_before,"after":resume_after,"relay":resumed}));
        assert!(!resume_after.active);
        assert_eq!(resume_after.lan_bytes, resume_before.lan_bytes);
        assert_eq!(resume_after.lan_frames, resume_before.lan_frames);
        assert_eq!(resume_after.suppressed, resume_before.suppressed);
        assert!(resume_after.relay_bytes > resume_before.relay_bytes);
        assert!(resume_after.relay_frames >= resume_before.relay_frames + 2);
        assert!(resumed.frames >= 2 && resumed.bytes > 0 && resumed.pong);

        config.running.store(false, Ordering::SeqCst);
        let _ = timeout(Duration::from_secs(5), relay.socket.close(None)).await;
        drop(relay);
        timeout(Duration::from_secs(12), relay_task).await.expect("relay stop timeout")
            .unwrap().expect("production relay session failed");
        evidence("passed", serde_json::json!({"host":snapshot(&config)}));
    }).await;
}


// Separate G2 regression: real authorized sockets, no manual active flag
// assignment, and no fake video statistics. Exercise abnormal TCP EOF,
// task cancellation, explicit LAN disable, and sharing stop in sequence.
#[tokio::test]
#[ignore = "requires interactive Windows desktop; mandatory G2 CI step"]
async fn windows_real_host_lan_exit_cleanup() {
    tokio::task::LocalSet::new().run_until(async {
        let relay_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let relay_address = relay_listener.local_addr().unwrap();
        let secret = protocol::new_secret();
        let (messages, logs) = mpsc::channel();
        let config = HostConfig {
            server: format!("ws://{relay_address}"),
            relay_token: "local-test-only".into(), secret,
            running: Arc::new(AtomicBool::new(true)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(true)),
            lan_active: Arc::new(AtomicBool::new(false)),
            control_gate: Arc::new(host::ControlGate::default()),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        let relay_config = config.clone();
        let relay_task = tokio::task::spawn_local(async move { host::run(&relay_config).await });
        let (stream, _) = timeout(Duration::from_secs(15), relay_listener.accept()).await.unwrap().unwrap();
        let socket = timeout(Duration::from_secs(10), accept_async(stream)).await.unwrap().unwrap();
        let mut relay = Peer::new(socket, secret, false);
        relay.send(protocol::PING, &100u64.to_be_bytes()).await;
        let baseline = relay.sample(6, 100).await;
        assert!(baseline.pong && baseline.frames > 0);
        g2_evidence("baseline", serde_json::json!({"host":snapshot(&config),"relay":baseline}));

        for (i, mode) in ["tcp_drop", "task_cancel", "lan_disable", "silent_peer", "sharing_stop"].into_iter().enumerate() {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let lan_config = config.clone();
            let task = tokio::task::spawn_local(async move {
                let (stream, address) = listener.accept().await.unwrap();
                let (socket, key) = accept_authorized(stream, &lan_config).await.unwrap();
                serve_authorized_session(&lan_config, socket, key, address.ip()).await;
            });
            let (mut socket, _) = timeout(Duration::from_secs(10),
                connect_async(format!("ws://{address}/direct"))).await.unwrap().unwrap();
            let client = protocol::new_secret();
            socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
                &transcript(HELLO_PAYLOAD, &client, &[])).unwrap())).await.unwrap();
            let (kind, challenge) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
            assert_eq!(kind, READY);
            let prefix = transcript(CHALLENGE_PAYLOAD, &client, &[]);
            let server = challenge.strip_prefix(prefix.as_slice()).unwrap();
            assert_eq!(server.len(), NONCE_LEN);
            assert!(!config.lan_active.load(Ordering::SeqCst));
            socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
                &transcript(PROOF_PAYLOAD, &client, server)).unwrap())).await.unwrap();
            let (kind, ready) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
            assert_eq!(kind, READY);
            assert_eq!(ready, transcript(READY_PAYLOAD, &client, server));
            let mut lan = Peer::new(socket, business_key(&secret, &client, server), true);
            let ping = 200 + i as u64;
            lan.send(protocol::PING, &ping.to_be_bytes()).await;
            let (settle, direct) = tokio::join!(relay.sample(4, 999), lan.sample(4, ping));
            let active = snapshot(&config);
            assert!(active.active && direct.pong && direct.frames > 0);
            g2_evidence("active", serde_json::json!({"mode":mode,"host":active,"lan":direct,"relay":settle}));

            let started = Instant::now();
            // For the silent-peer case keep the underlying TCP socket open and
            // drain real bytes without processing WebSocket frames (no Pong).
            // This is a responsive TCP stack with a silent WebSocket peer,
            // not a physical network outage or an Android fallback test.
            if mode == "silent_peer" {
                use tokio::io::AsyncReadExt;
                let _ = logs.try_iter().count();
                let mut raw_bytes = 0u64;
                let drain = async {
                    let mut buf = [0u8; 16384];
                    loop {
                        match lan.socket.get_mut().read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => raw_bytes += n as u64,
                            Err(_) => break,
                        }
                    }
                };
                let (joined, drained) = tokio::join!(
                    timeout(Duration::from_secs(25), task),
                    timeout(Duration::from_secs(26), drain)
                );
                joined.expect("silent LAN peer was not expired").unwrap();
                drained.expect("host did not close silent LAN socket");
                let released = snapshot(&config);
                let heartbeat_timeout = logs.try_iter().any(|s| s.contains("LAN heartbeat pong timed out"));
                g2_evidence("silent_peer_released", serde_json::json!({"host":released,
                    "observed_cleanup_ms":started.elapsed().as_millis(),
                    "raw_tcp_bytes_drained":raw_bytes,"heartbeat_timeout":heartbeat_timeout}));
                assert!(!released.active && heartbeat_timeout && raw_bytes > 0);
                drop(lan);
                let ping = 350 + i as u64;
                relay.send(protocol::PING, &ping.to_be_bytes()).await;
                let resumed = relay.sample(6, ping).await;
                let after = snapshot(&config);
                g2_evidence("resumed", serde_json::json!({"mode":mode,"before":released,
                    "after":after,"relay":resumed}));
                assert!(!after.active && resumed.pong && resumed.frames > 0);
                assert!(after.relay_bytes > released.relay_bytes && after.relay_frames > released.relay_frames);
                assert_eq!(after.lan_bytes, released.lan_bytes);
                assert_eq!(after.lan_frames, released.lan_frames);
                continue;
            }
            // Drop without WebSocket close, or cancel while the real session
            // owns the active lease. Disable/stop keeps the peer alive until
            // the production shutdown path has completed.
            let mut lan = Some(lan);
            match mode {
                "tcp_drop" => { drop(lan.take()); }
                "task_cancel" => { task.abort(); drop(lan.take()); }
                "lan_disable" => config.lan_direct.store(false, Ordering::SeqCst),
                "sharing_stop" => config.running.store(false, Ordering::SeqCst),
                _ => unreachable!(),
            }
            let joined = timeout(Duration::from_secs(12), task).await.expect("LAN exit timeout");
            if mode == "task_cancel" {
                assert!(joined.unwrap_err().is_cancelled());
            } else {
                joined.unwrap();
            }
            drop(lan);
            let released = snapshot(&config);
            assert!(!released.active, "LAN lease stuck after {}", mode);
            g2_evidence("released", serde_json::json!({"mode":mode,"host":released,
                "observed_cleanup_ms":started.elapsed().as_millis()}));
            if mode == "sharing_stop" { break; }

            config.lan_direct.store(true, Ordering::SeqCst);
            let ping = 300 + i as u64;
            relay.send(protocol::PING, &ping.to_be_bytes()).await;
            let resumed = relay.sample(6, ping).await;
            let after = snapshot(&config);
            g2_evidence("resumed", serde_json::json!({"mode":mode,"before":released,
                "after":after,"relay":resumed}));
            assert!(!after.active && resumed.pong && resumed.frames > 0);
            assert!(after.relay_bytes > released.relay_bytes);
            assert!(after.relay_frames > released.relay_frames);
            assert_eq!(after.lan_bytes, released.lan_bytes);
            assert_eq!(after.lan_frames, released.lan_frames);
        }
        timeout(Duration::from_secs(12), relay_task).await.expect("sharing stop timeout")
            .unwrap().expect("relay stop failed");
        drop(relay);
        g2_evidence("passed", serde_json::json!({"host":snapshot(&config)}));
    }).await;
}

fn g2_evidence(stage: &str, data: serde_json::Value) {
    use std::io::Write;
    let line = serde_json::json!({"stage":stage,"scope":"windows_runner_loopback_lan_cleanup","data":data}).to_string();
    eprintln!("{}", line);
    if let Ok(path) = std::env::var("P2P_G2_REPORT") {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)
            .expect("open G2 evidence");
        writeln!(file, "{}", line).expect("write G2 evidence");
    }
}


// Independent real OS input test; a small native EDIT window is owned by the
// test process, not an existing user's application. No hooks or keylogging.
mod input_probe {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;
    #[repr(C)]
    struct Point { x: i32, y: i32 }
    #[repr(C)]
    struct Rect { left: i32, top: i32, right: i32, bottom: i32 }
    #[repr(C)]
    struct Msg { hwnd: Hwnd, message: u32, wparam: usize, lparam: isize, time: u32, pt: Point, private: u32 }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32,
            x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: Hwnd,
            instance: Hwnd, param: *mut c_void) -> Hwnd;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn SetFocus(hwnd: Hwnd) -> Hwnd;
        fn GetFocus() -> Hwnd;
        fn GetWindowTextW(hwnd: Hwnd, out: *mut u16, max: i32) -> i32;
        fn GetWindowRect(hwnd: Hwnd, out: *mut Rect) -> i32;
        fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetCursorPos(out: *mut Point) -> i32;
        fn mouse_event(flags: u32, dx: u32, dy: u32, data: u32, extra: usize);
    }
    pub struct Window(Hwnd);
    impl Window {
        pub fn new() -> Self {
            let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
            let title = [0u16];
            let screen = screenshots::Screen::all().unwrap().remove(0).display_info;
            // A topmost, visible, multiline EDIT; coordinates inside it
            // are derived from the actual window rectangle below.
            let hwnd = unsafe { CreateWindowExW(8, class.as_ptr(), title.as_ptr(),
                0x90c00004, screen.x + 40, screen.y + 40, 420, 240,
                std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            assert!(!hwnd.is_null(), "native EDIT window creation failed");
            let w = Self(hwnd);
            unsafe { ShowWindow(hwnd, 5); SetForegroundWindow(hwnd); SetFocus(hwnd); }
            w.pump();
            w
        }
        pub fn pump(&self) {
            unsafe {
                let mut msg: Msg = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        pub fn focused(&self) -> bool {
            unsafe { GetForegroundWindow() == self.0 && GetFocus() == self.0 }
        }
        pub fn text(&self) -> String {
            self.pump();
            let mut out = [0u16; 128];
            let n = unsafe { GetWindowTextW(self.0, out.as_mut_ptr(), out.len() as i32) };
            String::from_utf16_lossy(&out[..n.max(0) as usize])
        }
        pub fn target(&self, offset: i32) -> (f64, f64, i32, i32) {
            let mut r = Rect { left:0, top:0, right:0, bottom:0 };
            assert_ne!(unsafe { GetWindowRect(self.0, &mut r) }, 0);
            let screen = screenshots::Screen::all().unwrap().remove(0).display_info;
            let x = r.left + 60 + offset;
            let y = r.top + 100;
            assert!(x < r.right && y < r.bottom);
            ((x-screen.x) as f64 / (screen.width-1) as f64,
             (y-screen.y) as f64 / (screen.height-1) as f64, x, y)
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            // Test-only emergency cleanup, also on assertion unwind.
            unsafe {
                if GetAsyncKeyState(1) < 0 { mouse_event(4,0,0,0,0); }
                DestroyWindow(self.0);
            }
        }
    }
    pub fn held() -> bool { unsafe { GetAsyncKeyState(1) < 0 } }
    pub fn at(x:i32,y:i32) -> bool {
        let mut p = Point{x:0,y:0};
        unsafe { GetCursorPos(&mut p) != 0 && (p.x-x).abs() <= 3 && (p.y-y).abs() <= 3 }
    }
}
async fn await_os(window: &input_probe::Window, label: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        window.pump();
        if check() { return; }
        assert!(Instant::now() < deadline, "OS assertion timed out: {}", label);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn settle_os(window: &input_probe::Window, millis: u64, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_millis(millis);
    loop {
        window.pump();
        assert!(check(), "unexpected OS input effect during blocked interval");
        if Instant::now() >= deadline { return; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn pointer<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut Peer<S>, kind: &str, target: (f64,f64,i32,i32)) {
    let data = serde_json::json!({"type":kind,"x":target.0,"y":target.1}).to_string();
    peer.send(protocol::CONTROL, data.as_bytes()).await;
}
fn input_evidence(stage: &str, data: serde_json::Value) {
    use std::io::Write;
    let line = serde_json::json!({"stage":stage,"scope":"windows_runner_real_sendinput_loopback","data":data}).to_string();
    eprintln!("{}", line);
    if let Ok(path) = std::env::var("P2P_G2_INPUT_REPORT") {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(f,"{}",line).unwrap();
    }
}
#[tokio::test]
#[ignore = "interactive Windows desktop; mandatory dedicated G2c input CI step"]
async fn windows_real_host_input_handoff() {
    tokio::task::LocalSet::new().run_until(async {
        let window = input_probe::Window::new();
        let a = window.target(0);
        let b = window.target(120);
        let c = window.target(240);
        assert!(!input_probe::held(), "left mouse already held before test");
        let relay_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let relay_address = relay_listener.local_addr().unwrap();
        let lan_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let lan_address = lan_listener.local_addr().unwrap();
        let secret = protocol::new_secret();
        let (messages, _logs) = mpsc::channel();
        let config = HostConfig {
            server: format!("ws://{relay_address}"),
            relay_token: "local-test-only".into(), secret,
            running: Arc::new(AtomicBool::new(true)),
            allow_input: Arc::new(AtomicBool::new(false)),
            allow_clipboard: Arc::new(AtomicBool::new(false)),
            monitor_index: Arc::new(AtomicUsize::new(0)),
            lan_direct: Arc::new(AtomicBool::new(true)),
            lan_active: Arc::new(AtomicBool::new(false)),
            control_gate: Arc::new(host::ControlGate::default()),
            allow_udp_probe: Arc::new(AtomicBool::new(false)),
            allow_udp_preview: Arc::new(AtomicBool::new(false)),
            udp_active: Arc::new(AtomicBool::new(false)),
            h264_enabled: Arc::new(AtomicBool::new(false)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        let relay_config = config.clone();
        let relay_task = tokio::task::spawn_local(async move { host::run(&relay_config).await });
        let (stream, _) = timeout(Duration::from_secs(15), relay_listener.accept())
            .await.expect("host relay connect timeout").unwrap();
        let socket = timeout(Duration::from_secs(10), accept_async(stream)).await.unwrap().unwrap();
        let mut relay = Peer::new(socket, secret, false);

        config.control_gate.set_input(&config.allow_input, true);
        pointer(&mut relay,"down",a).await;
        relay.service_until(await_os(&window,"relay real down",||input_probe::held() && input_probe::at(a.2,a.3) && window.focused())).await;
        pointer(&mut relay,"move",b).await;
        relay.service_until(await_os(&window,"relay real drag",||input_probe::held() && input_probe::at(b.2,b.3))).await;
        input_evidence("relay_drag",serde_json::json!({"down":true,"moved":true}));
        let lan_config = config.clone();
        let lan_task = tokio::task::spawn_local(async move {
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip()).await;
        });
        let (mut socket, _) = timeout(Duration::from_secs(10),
            connect_async(format!("ws://{lan_address}/direct"))).await.unwrap().unwrap();
        let client = protocol::new_secret();
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(HELLO_PAYLOAD, &client, &[])).unwrap())).await.unwrap();
        let (kind, challenge) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        let prefix = transcript(CHALLENGE_PAYLOAD, &client, &[]);
        let server = challenge.strip_prefix(prefix.as_slice()).unwrap();
        assert_eq!(server.len(), NONCE_LEN);
        assert!(!config.lan_active.load(Ordering::SeqCst), "LAN active before proof");
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(PROOF_PAYLOAD, &client, server)).unwrap())).await.unwrap();
        let (kind, ready) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        assert_eq!(ready, transcript(READY_PAYLOAD, &client, server));
        let mut lan = Peer::new(socket, business_key(&secret, &client, server), true);

        await_os(&window,"release relay drag at LAN activation",||!input_probe::held()).await;
        input_evidence("lan_takeover",serde_json::json!({"relay_button_released":true}));
        let _ = tokio::join!(relay.sample(3,999),lan.sample(3,999));
        pointer(&mut lan,"down",a).await;
        lan.service_until(await_os(&window,"LAN real down",||input_probe::held() && input_probe::at(a.2,a.3))).await;
        pointer(&mut relay,"up",c).await;
        pointer(&mut relay,"move",c).await;
        relay.send(protocol::CONTROL,br#"{"type":"text","text":"BLOCKED"}"#).await;
        // Ensure the competing relay has time to be serviced while the
        // LAN button remains held. Both transport sockets are drained.
        let (r,l,_) = tokio::join!(relay.sample(2,999),lan.sample(2,999),
            settle_os(&window,1800,||input_probe::held() && input_probe::at(a.2,a.3) && window.text().is_empty()));
        assert_eq!(r.frames,0);
        assert!(l.frames>0);
        pointer(&mut lan,"move",b).await;
        lan.service_until(await_os(&window,"LAN real drag",||input_probe::held() && input_probe::at(b.2,b.3))).await;
        pointer(&mut lan,"up",b).await;
        lan.service_until(await_os(&window,"LAN real up",||!input_probe::held())).await;
        assert!(window.focused());
        lan.send(protocol::CONTROL,br#"{"type":"text","text":"LAN_OK"}"#).await;
        lan.service_until(await_os(&window,"LAN real Unicode text",||window.text()=="LAN_OK")).await;
        assert!(window.focused(), "test EDIT lost focus before key command");
        lan.send(protocol::CONTROL,br#"{"type":"key","key":"end"}"#).await;
        lan.send(protocol::CONTROL,br#"{"type":"key","key":"backspace"}"#).await;
        lan.service_until(await_os(&window,"LAN real keyboard backspace",||window.text()=="LAN_O")).await;
        input_evidence("lan_exclusive",serde_json::json!({"relay_move_up_text_blocked":true,"lan_drag":true,"unicode":true,"key":true}));
        pointer(&mut lan,"down",a).await;
        lan.service_until(await_os(&window,"LAN held at disconnect",||input_probe::held())).await;
        // Abrupt peer drop must release the actual OS button, not just a flag.
        drop(lan);
        timeout(Duration::from_secs(12),lan_task).await.unwrap().unwrap();
        await_os(&window,"LAN disconnect real release",||!input_probe::held()).await;
        assert!(!config.lan_active.load(Ordering::SeqCst));
        pointer(&mut relay,"down",c).await;
        relay.service_until(await_os(&window,"relay resumed real down",||input_probe::held() && input_probe::at(c.2,c.3))).await;
        config.control_gate.set_input(&config.allow_input, false);
        relay.service_until(await_os(&window,"revocation real release",||!input_probe::held())).await;
        pointer(&mut relay,"down",a).await;
        relay.send(protocol::CONTROL,br#"{"type":"text","text":"DENIED"}"#).await;
        let before = window.text();
        let (_,_) = tokio::join!(relay.sample(2,999),
            settle_os(&window,1800,||!input_probe::held() && input_probe::at(c.2,c.3) && window.text()==before));
        config.control_gate.set_input(&config.allow_input, true);
        pointer(&mut relay,"down",b).await;
        relay.service_until(await_os(&window,"reauthorized relay real down",||input_probe::held() && input_probe::at(b.2,b.3))).await;
        // No await between local permission transitions: exercise the race
        // faster than the input worker's 50ms poll. Actual button must release.
        config.control_gate.set_input(&config.allow_input, false);
        config.control_gate.set_input(&config.allow_input, true);
        relay.service_until(await_os(&window,"rapid revoke/regrant real release",||!input_probe::held())).await;
        input_evidence("rapid_permission_cycle",serde_json::json!({"released":true}));
        pointer(&mut relay,"down",b).await;
        relay.service_until(await_os(&window,"fresh post-cycle down",||input_probe::held() && input_probe::at(b.2,b.3))).await;
        config.control_gate.stop(&config.running);
        timeout(Duration::from_secs(12),relay_task).await.unwrap().unwrap().unwrap();
        await_os(&window,"sharing stop real release",||!input_probe::held()).await;
        drop(relay);
        input_evidence("passed",serde_json::json!({"lan_disconnect_release":true,"relay_resumed":true,
            "revocation_release":true,"denied_input_blocked":true,"reauthorized":true,"stop_release":true}));
    }).await;
}

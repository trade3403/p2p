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
    relay_socket_open: bool,
    relay_socket_opens: u64,
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
        relay_socket_open: s.relay_open_sockets.load(Ordering::SeqCst) > 0,
        relay_socket_opens: s.relay_socket_opens.load(Ordering::Relaxed),
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
        assert!(after.relay_socket_open && after.relay_socket_opens == 1);
        assert!(after.relay_bytes > before.relay_bytes && after.relay_frames >= before.relay_frames + 2);
        assert!(baseline.frames >= 2 && baseline.bytes > 0 && baseline.pong);

        // Use the exact production authorization and activation functions,
        // never assign lan_active or increment host video counters in the test.
        let lan_config = config.clone();
        let lan_task = tokio::task::spawn_local(async move {
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key, lease) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip(), lease).await;
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
        assert!(stable_after.relay_socket_open && stable_after.relay_socket_opens == 1);
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
        assert!(resume_after.relay_socket_open && resume_after.relay_socket_opens == 1);
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
        let stopped = snapshot(&config);
        assert!(!stopped.relay_socket_open && stopped.relay_socket_opens == 1);
        evidence("passed", serde_json::json!({"host":stopped}));
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
                let (socket, key, lease) = accept_authorized(stream, &lan_config).await.unwrap();
                serve_authorized_session(&lan_config, socket, key, address.ip(), lease).await;
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
    exercise_real_input_handoff(false).await;
}

#[tokio::test]
#[ignore = "interactive Windows desktop; mandatory dedicated G5d-3b CI step"]
async fn windows_real_host_staged_input_handoff() {
    exercise_real_input_handoff(true).await;
}

async fn exercise_real_input_handoff(staged: bool) {
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
            if staged {
                let (stream, _) = lan_listener.accept().await.unwrap();
                assert!(accept_authorized(stream, &lan_config).await.is_err(),
                    "cancelled PREPARED must not authorize");
            }
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key, lease) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip(), lease).await;
        });
        let version = if staged { HandshakeVersion::V5 } else { HandshakeVersion::V4 };
        if staged {
            let before = snapshot(&config);
            let epoch = config.control_gate.video_epoch().load(Ordering::SeqCst);
            let (mut cancelled, _) = timeout(Duration::from_secs(10),
                connect_async(format!("ws://{lan_address}/direct"))).await.unwrap().unwrap();
            let client = protocol::new_secret();
            cancelled.send(Message::Binary(protocol::encrypt(&secret, HELLO,
                &transcript(HELLO_V5, &client, &[])).unwrap())).await.unwrap();
            let (kind, challenge) = protocol::decrypt(&secret, &binary(&mut cancelled).await).unwrap();
            assert_eq!(kind, READY);
            let prefix = transcript(CHALLENGE_V5, &client, &[]);
            let server = challenge.strip_prefix(prefix.as_slice()).unwrap();
            assert_eq!(server.len(), NONCE_LEN);
            cancelled.send(Message::Binary(protocol::encrypt(&secret, HELLO,
                &transcript(PROOF_V5, &client, server)).unwrap())).await.unwrap();
            let (kind, prepared) = protocol::decrypt(&secret, &binary(&mut cancelled).await).unwrap();
            assert_eq!((kind, prepared), (READY, transcript(PREPARED_V5, &client, server)));
            assert!(!snapshot(&config).active);
            pointer(&mut relay,"move",c).await;
            relay.service_until(await_os(&window,"relay drag while candidate prepared",||input_probe::held() && input_probe::at(c.2,c.3))).await;
            drop(cancelled);
            pointer(&mut relay,"move",b).await;
            relay.service_until(await_os(&window,"relay drag after candidate cancel",||input_probe::held() && input_probe::at(b.2,b.3))).await;
            relay.send(protocol::PING,&71u64.to_be_bytes()).await;
            let observed = relay.sample(2,71).await;
            let after = snapshot(&config);
            assert!(observed.frames>0 && observed.pong);
            assert!(!after.active && after.relay_socket_open && after.relay_socket_opens==1);
            assert!(after.relay_bytes>before.relay_bytes && after.lan_bytes==before.lan_bytes);
            assert_eq!(config.control_gate.video_epoch().load(Ordering::SeqCst),epoch);
            assert!(input_probe::held());
            input_evidence("v5_cancelled",serde_json::json!({"before":before,"after":after,
                "relay":observed,"relay_drag_preserved":true,"epoch_unchanged":true}));
        }
        let (mut socket, _) = timeout(Duration::from_secs(10),
            connect_async(format!("ws://{lan_address}/direct"))).await.unwrap().unwrap();
        let client = protocol::new_secret();
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(version.hello(), &client, &[])).unwrap())).await.unwrap();
        let (kind, challenge) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        let prefix = transcript(version.challenge(), &client, &[]);
        let server = challenge.strip_prefix(prefix.as_slice()).unwrap();
        assert_eq!(server.len(), NONCE_LEN);
        assert!(!config.lan_active.load(Ordering::SeqCst), "LAN active before proof");
        socket.send(Message::Binary(protocol::encrypt(&secret, HELLO,
            &transcript(version.proof(), &client, server)).unwrap())).await.unwrap();
        if staged {
            let (kind, prepared) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
            assert_eq!((kind,prepared),(READY,transcript(PREPARED_V5,&client,server)));
            assert!(!snapshot(&config).active && input_probe::held());
            pointer(&mut relay,"move",a).await;
            relay.service_until(await_os(&window,"relay drag before explicit commit",||input_probe::held() && input_probe::at(a.2,a.3))).await;
            assert!(!snapshot(&config).active);
            input_evidence("v5_prepared",serde_json::json!({"host":snapshot(&config),"relay_drag_preserved":true}));
            socket.send(Message::Binary(protocol::encrypt(&secret,HELLO,
                &transcript(COMMIT_V5,&client,server)).unwrap())).await.unwrap();
        }
        let (kind, ready) = protocol::decrypt(&secret, &binary(&mut socket).await).unwrap();
        assert_eq!(kind, READY);
        assert_eq!(ready, transcript(version.ready(), &client, server));
        if staged { assert!(snapshot(&config).active); }
        let mut lan = Peer::new(socket, versioned_business_key(&secret, &client, server,version), true);

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
        let stable_before = snapshot(&config);
        let (r,l,_) = tokio::join!(relay.sample(2,999),lan.sample(2,999),
            settle_os(&window,1800,||input_probe::held() && input_probe::at(a.2,a.3) && window.text().is_empty()));
        assert_eq!(r.frames,0);
        assert!(l.frames>0);
        if staged {
            let after = snapshot(&config);
            assert!(after.active && after.relay_socket_open && after.relay_socket_opens==1);
            assert_eq!(after.relay_bytes,stable_before.relay_bytes);
            assert_eq!(after.relay_frames,stable_before.relay_frames);
            assert!(after.lan_bytes>stable_before.lan_bytes && after.suppressed>stable_before.suppressed);
            input_evidence("v5_video_exclusive",serde_json::json!({"before":stable_before,"after":after,"relay":r,"lan":l}));
        }
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
        if staged {
            let before = snapshot(&config);
            relay.send(protocol::PING,&72u64.to_be_bytes()).await;
            let observed = relay.sample(3,72).await;
            let after = snapshot(&config);
            assert!(observed.frames>0 && observed.pong);
            assert!(!after.active && after.relay_socket_open && after.relay_socket_opens==1);
            assert!(after.relay_bytes>before.relay_bytes && after.lan_bytes==before.lan_bytes);
            input_evidence("v5_relay_resumed",serde_json::json!({"before":before,"after":after,"relay":observed}));
        }
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


// G5b-1: observe real negotiated Annex-B access units, never inject an AVC
// fixture into the Host. Keep the JPEG-only G1/G2 observation unchanged.
#[derive(Default, Debug, serde::Serialize)]
struct AvcObserved {
    elapsed_ms: u64,
    timing_samples: Vec<SessionTimingObservation>,
    bytes: u64,
    packets: u64,
    vcl_packets: u64,
    jpeg_packets: u64,
    sps_packets: u64,
    pps_packets: u64,
    idr_packets: u64,
    decoded_idr: u64,
    decoded_vcl: u64,
    decoded_predictive: u64,
    first_decoded_idr_ms: Option<u64>,
    dimensions: Option<(usize, usize)>,
    pong: bool,
    ws_pings: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionTiming {
    capture: crate::video::TimingSample,
    encode: crate::video::TimingSample,
    send: crate::video::TimingSample,
    queue_wait: crate::video::TimingSample,
    queue_drops: u64,
}
#[derive(Debug, serde::Serialize)]
struct SessionTimingObservation {
    observed_ms: u64,
    encoder: String,
    capture_backend: String,
    timing: SessionTiming,
}

fn timing_interval(samples: &[SessionTimingObservation]) -> serde_json::Value {
    let first = samples.first().expect("session timing missing");
    let last = samples.last().unwrap();
    assert!(samples.len() >= 2 && last.observed_ms > first.observed_ms);
    let stage = |a: crate::video::TimingSample, b: crate::video::TimingSample| {
        let d = b.delta(a).expect("session timing regressed");
        assert!(d.samples > 0, "no stage progress");
        serde_json::json!({"total_us":d.total_us,"samples":d.samples,
            "mean_us":d.total_us as f64 / d.samples as f64})
    };
    for pair in samples.windows(2) {
        let (a,b) = (&pair[0].timing,&pair[1].timing);
        assert!(b.capture.delta(a.capture).is_some() && b.encode.delta(a.encode).is_some()
            && b.send.delta(a.send).is_some() && b.queue_wait.delta(a.queue_wait).is_some()
            && b.queue_drops >= a.queue_drops, "session timing counters regressed");
    }
    let (a,b) = (&first.timing,&last.timing);
    assert_eq!(a.send.samples,a.queue_wait.samples);
    assert_eq!(b.send.samples,b.queue_wait.samples);
    serde_json::json!({
        "elapsed_observation_ms":last.observed_ms-first.observed_ms,
        "metadata_samples":samples.len(),
        "capture":stage(a.capture,b.capture),"encode":stage(a.encode,b.encode),
        "send":stage(a.send,b.send),"queue_wait":stage(a.queue_wait,b.queue_wait),
        "queue_drops":b.queue_drops-a.queue_drops,
        "encoder_first":first.encoder,"encoder_last":last.encoder,
        "capture_backend_first":first.capture_backend,"capture_backend_last":last.capture_backend
    })
}

fn avc_nal_types(bytes: &[u8]) -> Vec<u8> {
    // Both 3- and 4-byte Annex-B prefixes are legal. Reject empty/invalid NALs.
    assert!(bytes.starts_with(&[0,0,1]) || bytes.starts_with(&[0,0,0,1]),
        "missing Annex-B start code");
    let types: Vec<u8> = openh264::nal_units(bytes).map(|nal| {
        let body = nal.strip_prefix(&[0,0,0,1]).or_else(|| nal.strip_prefix(&[0,0,1]))
            .expect("invalid Annex-B prefix");
        let header = *body.first().expect("empty AVC NAL");
        assert_eq!(header & 0x80, 0, "forbidden AVC NAL bit");
        let kind = header & 31;
        assert!((1..=23).contains(&kind), "invalid AVC NAL type");
        kind
    }).collect();
    assert!(!types.is_empty(), "no AVC NAL units");
    types
}

#[test]
fn g5b_annex_b_nal_types() {
    // Structural parser test only; these bytes are not performance evidence.
    assert_eq!(avc_nal_types(&[0,0,0,1,0x67,2,0,0,1,0x68,3,0,0,0,1,0x65,4]),
        vec![7,8,5]);
}

impl<S: AsyncRead + AsyncWrite + Unpin> Peer<S> {
    async fn sample_avc(&mut self, seconds: u64, expected_ping: u64,
        expected_size: (usize, usize)) -> AvcObserved {
        self.observe_avc(seconds, expected_ping, expected_size, false, false).await
    }

    async fn observe_avc(&mut self, seconds: u64, expected_ping: u64,
        expected_size: (usize, usize), continuous: bool, require_first_idr: bool) -> AvcObserved {
        use openh264::formats::YUVSource;
        let mut stream_decoder: Option<openh264::decoder::Decoder> = None;
        let started = Instant::now();
        let deadline = started + Duration::from_secs(seconds);
        let mut result = AvcObserved::default();
        loop {
            let message = match timeout_at(deadline, self.socket.next()).await {
                Err(_) => break,
                Ok(Some(Ok(message))) => message,
                _ => panic!("peer disconnected during AVC observation"),
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
                            assert_eq!(result.packets, 0, "JPEG after negotiated AVC");
                            result.jpeg_packets += 1;
                        }
                        protocol::AVC => {
                            assert!(payload.len() > 4 && payload.len() <= 2 * 1024 * 1024);
                            let size = (u16::from_be_bytes([payload[0],payload[1]]) as usize,
                                u16::from_be_bytes([payload[2],payload[3]]) as usize);
                            assert_eq!(size, expected_size, "AVC header differs from real display profile");
                            result.dimensions = Some(size);
                            let types = avc_nal_types(&payload[4..]);
                            let (sps, pps, idr) = (types.contains(&7), types.contains(&8), types.contains(&5));
                            result.sps_packets += u64::from(sps);
                            result.pps_packets += u64::from(pps);
                            result.idr_packets += u64::from(idr);
                            if types.contains(&1) || idr { result.vcl_packets += 1; }
                            if require_first_idr && result.packets == 0 {
                                assert!(sps && pps && idr, "first resumed AVC was not a self-contained IDR");
                                assert_eq!(result.jpeg_packets, 0, "unexpected JPEG on negotiated AVC resume");
                            }
                            if continuous {
                                if stream_decoder.is_none() && sps && pps && idr {
                                    stream_decoder = Some(openh264::decoder::Decoder::new().expect("stream decoder"));
                                }
                                if let Some(decoder) = stream_decoder.as_mut() {
                                    let frame = decoder.decode(&payload[4..]).expect("continuous AVC decode failed");
                                    if types.contains(&1) || idr {
                                        let frame = frame.expect("VCL did not yield a decoded picture");
                                        assert_eq!(frame.dimensions(), size);
                                        result.decoded_vcl += 1;
                                        if !idr { result.decoded_predictive += 1; }
                                    }
                                }
                            }
                            // Independently decode the first self-contained random access
                            // unit in this window; don't rely on earlier decoder state.
                            if sps && pps && idr && result.decoded_idr == 0 {
                                let mut decoder = openh264::decoder::Decoder::new().expect("test decoder");
                                let decoded = decoder.decode(&payload[4..]).expect("invalid IDR bitstream")
                                    .expect("IDR did not produce a decoded frame");
                                assert_eq!(decoded.dimensions(), size, "decoded dimensions differ from wire header");
                                assert!(!decoded.y().is_empty(), "empty decoded luma");
                                result.decoded_idr = 1;
                                result.first_decoded_idr_ms = Some(started.elapsed().as_millis() as u64);
                            }
                            result.packets += 1;
                            result.bytes += packet.len() as u64;
                        }
                        protocol::MONITORS => {
                            let metadata: serde_json::Value = serde_json::from_slice(&payload).expect("host metadata");
                            let timing = serde_json::from_value(metadata["session_video_timing"].clone())
                                .expect("host session video timing");
                            result.timing_samples.push(SessionTimingObservation {
                                observed_ms: started.elapsed().as_millis() as u64,
                                encoder: metadata["video_encoder"].as_str().unwrap_or("unknown").to_owned(),
                                capture_backend: metadata["capture_backend"].as_str().unwrap_or("unknown").to_owned(),
                                timing,
                            });
                        }
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
                Message::Close(_) => panic!("host closed during AVC observation"),
                _ => {}
            }
            if Instant::now() >= deadline { break; }
        }
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        result
    }
}

fn g5b_evidence(stage: &str, data: serde_json::Value) {
    use std::io::Write;
    let line = serde_json::json!({"stage":stage,"scope":"windows_runner_loopback_ws_real_avc","data":data}).to_string();
    eprintln!("{}", line);
    if let Ok(path) = std::env::var("P2P_G5B_REPORT") {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)
            .expect("open G5b evidence");
        writeln!(file, "{}", line).expect("write G5b evidence");
    }
}

#[tokio::test]
#[ignore = "requires interactive Windows desktop; mandatory dedicated G5b-1 Windows CI step"]
async fn windows_real_host_lan_avc_and_relay_recovery() {
    // The production host runs in a current-thread runtime. Local tasks avoid
    // imposing Send on its platform-specific native input/clipboard objects.
    tokio::task::LocalSet::new().run_until(async {
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
            h264_enabled: Arc::new(AtomicBool::new(true)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        g5b_evidence("start", serde_json::json!({"host":snapshot(&config)}));
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
        g5b_evidence("relay_baseline", serde_json::json!({"before":before,"after":after,"relay":baseline}));
        assert!(!after.active && after.lan_frames == 0 && after.lan_bytes == 0);
        assert!(after.relay_socket_open && after.relay_socket_opens == 1);
        assert!(after.relay_bytes > before.relay_bytes && after.relay_frames >= before.relay_frames + 2);
        assert!(baseline.frames >= 2 && baseline.bytes > 0 && baseline.pong);

        // Use the exact production authorization and activation functions,
        // never assign lan_active or increment host video counters in the test.
        let lan_config = config.clone();
        let lan_task = tokio::task::spawn_local(async move {
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key, lease) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip(), lease).await;
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
        let display = screenshots::Screen::all().unwrap().remove(0).display_info;
        let (w,h) = crate::video::dimensions(display.width, display.height);
        assert!(w >= 2 && h >= 2);
        let size = (w as usize,h as usize);
        lan.send(protocol::CONTROL, br#"{"type":"video_codec","codec":"h264"}"#).await;
        lan.send(protocol::PING, &2u64.to_be_bytes()).await;
        // Allow bounded startup negotiation and any queued JPEG to drain.
        let (drain, startup) = tokio::join!(relay.sample(10,99),lan.sample_avc(10,2,size));
        let stable_before = snapshot(&config);
        g5b_evidence("lan_startup",serde_json::json!({"host":stable_before,"relay":drain,"lan":startup}));
        assert!(stable_before.active && startup.pong && startup.vcl_packets >= 2);
        assert!(startup.sps_packets > 0 && startup.pps_packets > 0 && startup.idr_packets > 0);
        assert_eq!(startup.decoded_idr,1, "no independently decoded LAN IDR");
        relay.send(protocol::PING, &3u64.to_be_bytes()).await;
        lan.send(protocol::PING, &4u64.to_be_bytes()).await;
        let (blocked, direct) = tokio::join!(relay.sample(10,3),lan.sample_avc(10,4,size));
        let stable_after = snapshot(&config);
        g5b_evidence("lan_stable",serde_json::json!({"before":stable_before,"after":stable_after,
            "relay":blocked,"lan":direct}));
        assert!(stable_after.active && stable_after.relay_socket_open && stable_after.relay_socket_opens == 1);
        assert_eq!((stable_after.relay_bytes,stable_after.relay_frames),
            (stable_before.relay_bytes,stable_before.relay_frames));
        assert_eq!((blocked.frames,blocked.bytes),(0,0));
        assert!(!blocked.pong && direct.pong && direct.ws_pings > 0);
        assert_eq!(direct.jpeg_packets,0);
        assert!(direct.vcl_packets >= 2 && direct.bytes > 0);
        assert!(stable_after.lan_bytes > stable_before.lan_bytes
            && stable_after.lan_frames >= stable_before.lan_frames + 2);
        assert!(stable_after.suppressed > stable_before.suppressed);

        // The baseline relay is JPEG. After actual LAN disconnect the same
        // authorized relay socket negotiates real AVC; continuity of an
        // already-negotiated relay AVC GOP is a separate G5b-2 requirement.
        timeout(Duration::from_secs(5),lan.socket.close(None)).await.unwrap().unwrap();
        drop(lan);
        timeout(Duration::from_secs(12),lan_task).await.expect("LAN cleanup timeout").unwrap();
        let resume_before = snapshot(&config);
        assert!(!resume_before.active);
        relay.send(protocol::CONTROL,br#"{"type":"video_codec","codec":"h264"}"#).await;
        relay.send(protocol::PING,&5u64.to_be_bytes()).await;
        let resumed = relay.sample_avc(10,5,size).await;
        let resume_after = snapshot(&config);
        g5b_evidence("relay_resumed",serde_json::json!({"before":resume_before,"after":resume_after,"relay":resumed}));
        assert!(!resume_after.active && resume_after.relay_socket_open && resume_after.relay_socket_opens == 1);
        assert_eq!((resume_after.lan_bytes,resume_after.lan_frames,resume_after.suppressed),
            (resume_before.lan_bytes,resume_before.lan_frames,resume_before.suppressed));
        assert!(resume_after.relay_bytes > resume_before.relay_bytes
            && resume_after.relay_frames >= resume_before.relay_frames + 2);
        assert!(resumed.pong && resumed.vcl_packets >= 2 && resumed.bytes > 0);
        assert!(resumed.sps_packets > 0 && resumed.pps_packets > 0 && resumed.idr_packets > 0);
        assert_eq!(resumed.decoded_idr,1,"no independently decoded relay IDR");
        config.running.store(false, Ordering::SeqCst);
        let _ = timeout(Duration::from_secs(5),relay.socket.close(None)).await;
        drop(relay);
        timeout(Duration::from_secs(12),relay_task).await.expect("relay stop timeout")
            .unwrap().expect("production relay session failed");
        let stopped = snapshot(&config);
        assert!(!stopped.relay_socket_open && stopped.relay_socket_opens == 1);
        g5b_evidence("passed",serde_json::json!({"host":stopped}));
    }).await;
}

fn g5b2_evidence(stage: &str, data: serde_json::Value) {
    use std::io::Write;
    let line = serde_json::json!({"stage":stage,"scope":"windows_runner_loopback_ws_avc_handoff","data":data}).to_string();
    eprintln!("{}", line);
    if let Ok(path) = std::env::var("P2P_G5B2_REPORT") {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)
            .expect("open G5b2 evidence");
        writeln!(file, "{}", line).expect("write G5b2 evidence");
    }
}

#[tokio::test]
#[ignore = "requires interactive Windows desktop; mandatory G5b-2 Windows CI step"]
async fn windows_real_host_negotiated_avc_handoff() {
    // The production host runs in a current-thread runtime. Local tasks avoid
    // imposing Send on its platform-specific native input/clipboard objects.
    tokio::task::LocalSet::new().run_until(async {
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
            h264_enabled: Arc::new(AtomicBool::new(true)),
            target_fps: Arc::new(AtomicU32::new(6)),
            sent_frames: Arc::new(AtomicU64::new(0)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            transport_video: Arc::new(host::TransportVideoStats::default()),
            messages,
        };
        g5b2_evidence("start", serde_json::json!({"host":snapshot(&config)}));
        let relay_config = config.clone();
        let relay_task = tokio::task::spawn_local(async move { host::run(&relay_config).await });
        let (stream, _) = timeout(Duration::from_secs(15), relay_listener.accept())
            .await.expect("host relay connect timeout").unwrap();
        let socket = timeout(Duration::from_secs(10), accept_async(stream)).await.unwrap().unwrap();
        let mut relay = Peer::new(socket, secret, false);
        let display = screenshots::Screen::all().unwrap().remove(0).display_info;
        let (w,h) = crate::video::dimensions(display.width, display.height);
        assert!(w >= 2 && h >= 2);
        let size = (w as usize,h as usize);
        let before = snapshot(&config);
        relay.send(protocol::CONTROL, br#"{"type":"video_codec","codec":"h264"}"#).await;
        relay.send(protocol::PING, &1u64.to_be_bytes()).await;
        let baseline = relay.observe_avc(10, 1, size, true, false).await;
        let after = snapshot(&config);
        g5b2_evidence("relay_baseline", serde_json::json!({"before":before,"after":after,"relay":baseline}));
        assert!(!after.active && after.lan_frames == 0 && after.lan_bytes == 0);
        assert!(after.relay_socket_open && after.relay_socket_opens == 1);
        assert!(after.relay_bytes > before.relay_bytes && after.relay_frames >= before.relay_frames + 2);
        assert!(baseline.decoded_vcl >= 2 && baseline.decoded_predictive > 0 && baseline.pong);

        // Use the exact production authorization and activation functions,
        // never assign lan_active or increment host video counters in the test.
        let lan_config = config.clone();
        let lan_task = tokio::task::spawn_local(async move {
            let (stream, address) = lan_listener.accept().await.unwrap();
            assert!(is_lan_address(address.ip()));
            let (socket, key, lease) = accept_authorized(stream, &lan_config).await.expect("LAN authorization");
            serve_authorized_session(&lan_config, socket, key, address.ip(), lease).await;
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
        lan.send(protocol::CONTROL, br#"{"type":"video_codec","codec":"h264"}"#).await;
        lan.send(protocol::PING, &2u64.to_be_bytes()).await;
        // Allow bounded startup negotiation and any queued JPEG to drain.
        let (drain, startup) = tokio::join!(relay.sample_avc(10,99,size),lan.observe_avc(10,2,size,true,false));
        let stable_before = snapshot(&config);
        g5b2_evidence("lan_startup",serde_json::json!({"host":stable_before,"relay":drain,"lan":startup}));
        assert!(stable_before.active && startup.pong && startup.vcl_packets >= 2);
        assert!(startup.sps_packets > 0 && startup.pps_packets > 0 && startup.idr_packets > 0);
        assert_eq!(startup.decoded_idr,1, "no independently decoded LAN IDR");
        assert!(startup.decoded_vcl >= 2 && startup.decoded_predictive > 0);
        relay.send(protocol::PING, &3u64.to_be_bytes()).await;
        lan.send(protocol::PING, &4u64.to_be_bytes()).await;
        let (blocked, direct) = tokio::join!(relay.sample_avc(10,3,size),lan.observe_avc(10,4,size,true,false));
        let stable_after = snapshot(&config);
        let timing = timing_interval(&direct.timing_samples);
        let performance = serde_json::json!({
            "scope":"windows_runner_loopback_real_capture_session_timing",
            "receiver_window_ms":direct.elapsed_ms,
            "received_avc_packets":direct.packets,
            "decoded_vcl":direct.decoded_vcl,
            "encrypted_avc_bytes":direct.bytes,
            "received_avc_packets_per_second":direct.packets as f64*1000.0/direct.elapsed_ms as f64,
            "decoded_vcl_per_second":direct.decoded_vcl as f64*1000.0/direct.elapsed_ms as f64,
            "encrypted_avc_bps":direct.bytes as f64*8000.0/direct.elapsed_ms as f64,
            "session_timing":timing
        });
        g5b2_evidence("lan_stable",serde_json::json!({"before":stable_before,"after":stable_after,
            "relay":blocked,"lan":direct,"performance":performance}));
        assert!(stable_after.active && stable_after.relay_socket_open && stable_after.relay_socket_opens == 1);
        assert_eq!((stable_after.relay_bytes,stable_after.relay_frames),
            (stable_before.relay_bytes,stable_before.relay_frames));
        assert_eq!((blocked.packets,blocked.jpeg_packets,blocked.bytes),(0,0,0));
        assert!(!blocked.pong && direct.pong && direct.ws_pings > 0);
        assert_eq!(direct.jpeg_packets,0);
        assert!(direct.vcl_packets >= 2 && direct.bytes > 0);
        assert!(direct.decoded_vcl >= 2 && direct.decoded_predictive > 0);
        assert!(stable_after.lan_bytes > stable_before.lan_bytes
            && stable_after.lan_frames >= stable_before.lan_frames + 2);
        assert!(stable_after.suppressed > stable_before.suppressed);

        // Abruptly drop LAN. Relay must resume without codec renegotiation.
        drop(lan);
        timeout(Duration::from_secs(12),lan_task).await.expect("LAN cleanup timeout").unwrap();
        let resume_before = snapshot(&config);
        assert!(!resume_before.active);
        relay.send(protocol::PING,&5u64.to_be_bytes()).await;
        let resumed = relay.observe_avc(10,5,size,true,true).await;
        let resume_after = snapshot(&config);
        g5b2_evidence("relay_resumed",serde_json::json!({"before":resume_before,"after":resume_after,"relay":resumed}));
        assert!(!resume_after.active && resume_after.relay_socket_open && resume_after.relay_socket_opens == 1);
        assert_eq!((resume_after.lan_bytes,resume_after.lan_frames,resume_after.suppressed),
            (resume_before.lan_bytes,resume_before.lan_frames,resume_before.suppressed));
        assert!(resume_after.relay_bytes > resume_before.relay_bytes
            && resume_after.relay_frames >= resume_before.relay_frames + 2);
        assert!(resumed.pong && resumed.vcl_packets >= 2 && resumed.bytes > 0);
        assert!(resumed.sps_packets > 0 && resumed.pps_packets > 0 && resumed.idr_packets > 0);
        assert_eq!(resumed.decoded_idr,1,"no independently decoded relay IDR");
        assert_eq!(resumed.decoded_vcl, resumed.vcl_packets);
        assert!(resumed.decoded_predictive > 0 && resumed.jpeg_packets == 0);
        config.running.store(false, Ordering::SeqCst);
        let _ = timeout(Duration::from_secs(5),relay.socket.close(None)).await;
        drop(relay);
        timeout(Duration::from_secs(12),relay_task).await.expect("relay stop timeout")
            .unwrap().expect("production relay session failed");
        let stopped = snapshot(&config);
        assert!(!stopped.relay_socket_open && stopped.relay_socket_opens == 1);
        g5b2_evidence("passed",serde_json::json!({"host":stopped}));
    }).await;
}

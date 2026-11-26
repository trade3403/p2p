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
        assert!(direct.frames >= 2 && direct.bytes > 0);

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

        for (i, mode) in ["tcp_drop", "task_cancel", "lan_disable", "sharing_stop"].into_iter().enumerate() {
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

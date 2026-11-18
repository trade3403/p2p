//! Windows Host-side one-time numeric pairing coordinator.
use crate::{host, pairing_api::{decode_base64url, PairingApiClient, PairingRequest}, pairing_crypto::{DeviceIdentity, PairingWindow}};
use anyhow::Result;
use subtle::ConstantTimeEq;
use std::{sync::{mpsc::{Receiver, RecvTimeoutError}, atomic::Ordering}, thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

#[derive(Debug, Clone)]
pub enum PairingDecision {
    Respond { request_id: String, approved: bool },
}

pub fn start(
    config: host::HostConfig,
    identity: DeviceIdentity,
    decisions: Receiver<PairingDecision>,
) {
    thread::spawn(move || {
        if let Err(error) = run(config.clone(), identity, decisions) {
            let _ = config.messages.send(format!("数字配对服务错误：{error:#}"));
            config.running.store(false, Ordering::SeqCst);
        }
    });
}

fn send_event(config: &host::HostConfig, value: serde_json::Value) {
    let _ = config.messages.send(value.to_string());
}

fn sleep_while_running(config: &host::HostConfig, duration: Duration) {
    let started = Instant::now();
    while config.running.load(Ordering::SeqCst) && started.elapsed() < duration {
        thread::sleep(Duration::from_millis(100));
    }
}

fn run(
    config: host::HostConfig,
    identity: DeviceIdentity,
    decisions: Receiver<PairingDecision>,
) -> Result<()> {
    let api = PairingApiClient::new(&config.server, &config.relay_token)?;
    let computer_name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Windows PC".to_owned());

    let device_number = loop {
        if !config.running.load(Ordering::SeqCst) { return Ok(()); }
        match api.register(&identity, &computer_name) {
            Ok(number) => break number,
            Err(error) => {
                send_event(&config, serde_json::json!({
                    "event":"pairing_status",
                    "message":format!("设备注册失败，5秒后重试：{error}")
                }));
                sleep_while_running(&config, Duration::from_secs(5));
            }
        }
    };

    send_event(&config, serde_json::json!({
        "event":"pairing_device",
        "device_number":device_number,
        "message":"设备已注册，正在生成动态验证码"
    }));

    let mut window: Option<PairingWindow> = None;
    let mut next_heartbeat = Instant::now();
    while config.running.load(Ordering::SeqCst) {
        let now_unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        let rotate = window.as_ref().map(|w| w.expires_at <= now_unix).unwrap_or(true);
        if rotate {
            let next = PairingWindow::generate(now_unix);
            match api.publish_window(&identity, &device_number, &next) {
                Ok(()) => {
                    send_event(&config, serde_json::json!({
                        "event":"pairing_code",
                        "device_number":device_number,
                        "code":next.code,
                        "expires_at":next.expires_at,
                        "message":"动态验证码已更新，有效期120秒"
                    }));
                    window = Some(next);
                }
                Err(error) => {
                    send_event(&config, serde_json::json!({
                        "event":"pairing_status",
                        "message":format!("发布动态验证码失败：{error}")
                    }));
                    sleep_while_running(&config, Duration::from_secs(2));
                    continue;
                }
            }
        }

        if next_heartbeat.elapsed() >= Duration::from_secs(20) {
            if let Err(error) = api.heartbeat(&identity) {
                send_event(&config, serde_json::json!({
                    "event":"pairing_status",
                    "message":format!("中转心跳暂时失败：{error}")
                }));
            }
            next_heartbeat = Instant::now();
        }

        let requests = match api.pending_requests(&identity) {
            Ok(requests) => requests,
            Err(error) => {
                send_event(&config, serde_json::json!({
                    "event":"pairing_status",
                    "message":format!("正在等待中转恢复：{error}")
                }));
                sleep_while_running(&config, Duration::from_secs(2));
                continue;
            }
        };

        for request in requests {
            if !config.running.load(Ordering::SeqCst) { break; }
            let Some(active_window) = window.as_ref() else { break; };
            let viewer_public = match decode_base64url(&request.viewer_public_key, 32) {
                Ok(value) => <[u8; 32]>::try_from(value).ok(),
                Err(_) => None,
            };
            let nonce = match decode_base64url(&request.nonce, 16) {
                Ok(value) => <[u8; 16]>::try_from(value).ok(),
                Err(_) => None,
            };
            let supplied_proof = match decode_base64url(&request.proof, 32) {
                Ok(value) => <[u8; 32]>::try_from(value).ok(),
                Err(_) => None,
            };
            let (Some(viewer_public), Some(nonce), Some(supplied_proof)) =
                (viewer_public, nonce, supplied_proof) else { continue; };
            let expected = match active_window.proof_for_viewer(&device_number, viewer_public, nonce) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if expected.ct_eq(&supplied_proof).unwrap_u8() != 1 {
                // Wrong codes are silently ignored. The Relay never learns the OTP.
                continue;
            }

            let fingerprint = hex::encode(&viewer_public[..6]);
            send_event(&config, serde_json::json!({
                "event":"pairing_request",
                "request_id":request.id,
                "device_number":device_number,
                "viewer_fingerprint":fingerprint,
                "message":"收到有效验证码，请确认是否允许这台设备连接"
            }));

            let seconds_left = (active_window.expires_at - now_unix).clamp(0, 120) as u64;
            let approval_deadline = Instant::now() + Duration::from_secs(seconds_left.min(45));
            let mut decision = None;
            while config.running.load(Ordering::SeqCst) && Instant::now() < approval_deadline {
                match decisions.recv_timeout(Duration::from_millis(250)) {
                    Ok(PairingDecision::Respond { request_id, approved }) if request_id == request.id => {
                        decision = Some(approved);
                        break;
                    }
                    Ok(_) => continue,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            let Some(approved) = decision else {
                let _ = api.decide(&identity, &request, false);
                send_event(&config, serde_json::json!({
                    "event":"pairing_request_expired",
                    "request_id":request.id,
                    "message":"连接确认超时或已取消"
                }));
                continue;
            };

            if let Err(error) = api.decide(&identity, &request, approved) {
                send_event(&config, serde_json::json!({
                    "event":"pairing_status",
                    "message":format!("提交配对决定失败：{error}")
                }));
                continue;
            }
            if !approved {
                send_event(&config, serde_json::json!({
                    "event":"pairing_request_denied",
                    "request_id":request.id,
                    "message":"已拒绝本次连接请求"
                }));
                continue;
            }

            let session_secret = active_window.session_secret(&device_number, viewer_public, nonce)?;
            let mut session_config = config.clone();
            session_config.secret = session_secret;
            send_event(&config, serde_json::json!({
                "event":"pairing_approved",
                "request_id":request.id,
                "message":"已批准配对，正在建立端到端加密远程会话"
            }));
            host::start(session_config);
            return Ok(());
        }

        sleep_while_running(&config, Duration::from_millis(500));
    }

    let _ = api.revoke_window(&identity);
    send_event(&config, serde_json::json!({
        "event":"pairing_stopped",
        "message":"数字配对已停止，动态验证码已撤销"
    }));
    Ok(())
}

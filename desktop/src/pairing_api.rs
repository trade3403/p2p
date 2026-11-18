//! HTTPS client for Relay device registration and numeric pairing.
use crate::pairing_crypto::{DeviceIdentity, PairingWindow};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct PairingRequest {
    pub id: String,
    pub device_id: String,
    pub device_number: String,
    pub viewer_public_key: String,
    pub nonce: String,
    pub proof: String,
    pub status: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceInfo {
    pub device_number: String,
    pub name: String,
    pub online: bool,
    pub identity_public_key: String,
    pub pairing_public_key: String,
    pub pairing_signature: String,
    pub pairing_expires_at: i64,
}

#[derive(Serialize)]
struct RegisterBody<'a> {
    device_id: &'a str,
    name: &'a str,
    identity_public_key: &'a str,
}

#[derive(Serialize)]
struct HeartbeatBody<'a> {
    device_id: &'a str,
}

#[derive(Serialize)]
struct WindowBody<'a> {
    device_id: &'a str,
    pairing_public_key: &'a str,
    signature: &'a str,
    expires_at: i64,
}

#[derive(Serialize)]
struct DecisionBody<'a> {
    device_id: &'a str,
    approved: bool,
    signature: &'a str,
}

#[derive(Serialize)]
struct CreateRequestBody<'a> {
    device_number: &'a str,
    viewer_public_key: &'a str,
    nonce: &'a str,
    proof: &'a str,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    device_number: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequestResponse {
    pub request_id: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct RequestStatus {
    pub status: String,
}

pub struct PairingApiClient {
    base_url: String,
    token: String,
    client: Client,
}

impl PairingApiClient {
    pub fn new(server: &str, token: &str) -> Result<Self> {
        anyhow::ensure!(token.len() >= 32, "relay token must contain at least 32 characters");
        let input = server.trim().trim_end_matches('/');
        let base_url = if let Some(host) = input.strip_prefix("wss://") {
            format!("https://{host}")
        } else if input.starts_with("https://") {
            input.to_owned()
        } else {
            anyhow::bail!("numeric pairing requires an HTTPS/WSS relay URL");
        };
        let parsed = url::Url::parse(&base_url).context("invalid relay URL")?;
        anyhow::ensure!(parsed.path() == "/" && parsed.query().is_none() && parsed.fragment().is_none(),
            "relay URL must contain only the scheme and host");
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("P2PRemote/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base_url, token: token.to_owned(), client })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client.request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn register(&self, identity: &DeviceIdentity, name: &str) -> Result<String> {
        let body = RegisterBody {
            device_id: &identity.device_id_hex(),
            name,
            identity_public_key: &identity.public_key_b64(),
        };
        let response: RegisterResponse = self.request(reqwest::Method::POST, "/api/v1/devices/register")
            .json(&body).send()?.error_for_status()?.json()?;
        anyhow::ensure!(response.device_number.len() == 9 &&
            response.device_number.bytes().all(|b| b.is_ascii_digit()),
            "relay returned an invalid device number");
        Ok(response.device_number)
    }

    pub fn heartbeat(&self, identity: &DeviceIdentity) -> Result<()> {
        let body = HeartbeatBody { device_id: &identity.device_id_hex() };
        self.request(reqwest::Method::POST, "/api/v1/devices/heartbeat")
            .json(&body).send()?.error_for_status()?;
        Ok(())
    }

    pub fn publish_window(
        &self, identity: &DeviceIdentity, device_number: &str, window: &PairingWindow,
    ) -> Result<()> {
        let public_key = window.public_key_b64();
        let signature = identity.sign_pairing_window(device_number, window.expires_at, &public_key);
        let body = WindowBody {
            device_id: &identity.device_id_hex(),
            pairing_public_key: &public_key,
            signature: &signature,
            expires_at: window.expires_at,
        };
        self.request(reqwest::Method::POST, "/api/v1/devices/pairing-window")
            .json(&body).send()?.error_for_status()?;
        Ok(())
    }

    pub fn pending_requests(&self, identity: &DeviceIdentity) -> Result<Vec<PairingRequest>> {
        let path = format!("/api/v1/pairing/requests?device_id={}", identity.device_id_hex());
        #[derive(Deserialize)]
        struct Response { requests: Vec<PairingRequest> }
        let response: Response = self.request(reqwest::Method::GET, &path)
            .send()?.error_for_status()?.json()?;
        Ok(response.requests)
    }

    pub fn decide(
        &self, identity: &DeviceIdentity, request: &PairingRequest, approved: bool,
    ) -> Result<()> {
        let signature = identity.sign_decision(
            &request.id, &request.device_number, &request.viewer_public_key, &request.nonce, approved,
        );
        let body = DecisionBody {
            device_id: &identity.device_id_hex(),
            approved,
            signature: &signature,
        };
        let path = format!("/api/v1/pairing/requests/{}/decision", request.id);
        self.request(reqwest::Method::POST, &path)
            .json(&body).send()?.error_for_status()?;
        Ok(())
    }

    pub fn lookup_device(&self, device_number: &str) -> Result<DeviceInfo> {
        anyhow::ensure!(device_number.len() == 9 && device_number.bytes().all(|b| b.is_ascii_digit()),
            "device number must contain nine digits");
        let device: DeviceInfo = self.request(
            reqwest::Method::GET, &format!("/api/v1/devices/{device_number}")
        ).send()?.error_for_status()?.json()?;
        Ok(device)
    }

    pub fn request_pairing(
        &self, device_number: &str, viewer_public_key: &[u8; 32],
        nonce: &[u8; 16], proof: &[u8; 32],
    ) -> Result<CreateRequestResponse> {
        let viewer_public_key = URL_SAFE_NO_PAD.encode(viewer_public_key);
        let nonce = URL_SAFE_NO_PAD.encode(nonce);
        let proof = URL_SAFE_NO_PAD.encode(proof);
        let body = CreateRequestBody {
            device_number, viewer_public_key: &viewer_public_key, nonce: &nonce, proof: &proof,
        };
        let response: CreateRequestResponse = self.request(reqwest::Method::POST, "/api/v1/pairing/requests")
            .json(&body).send()?.error_for_status()?.json()?;
        Ok(response)
    }

    pub fn request_status(&self, request_id: &str) -> Result<RequestStatus> {
        anyhow::ensure!(request_id.len() == 32 && request_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid pairing request ID");
        self.request(reqwest::Method::GET, &format!("/api/v1/pairing/requests/{request_id}"))
            .send()?.error_for_status()?.json().map_err(Into::into)
    }
}

pub fn decode_base64url(value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| anyhow!("invalid Base64URL value"))?;
    anyhow::ensure!(bytes.len() == expected_len, "invalid decoded key length");
    Ok(bytes)
}

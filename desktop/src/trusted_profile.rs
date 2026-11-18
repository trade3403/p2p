//! DPAPI-backed trusted profile. Scope: current Windows user, same machine.
//! No credentials are saved without a deliberate in-app action.
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf, process::Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedProfile {
    pub relay_url: String,
    pub relay_token: String,
    pub session_hex: String,
    pub launch_on_login: bool,
    pub unattended_input: bool,
}

fn vault_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is unset"))?;
    Ok(PathBuf::from(base).join("P2PRemote").join("trusted.bin"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn valid_profile(profile: &TrustedProfile) -> bool {
    if !crate::local_settings::is_safe_address(&profile.relay_url) {
        return false;
    }
    profile.relay_token.len() >= 32 && profile.relay_token.len() <= 512
        && profile.session_hex.len() == 64
        && profile.session_hex.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn load() -> io::Result<Option<TrustedProfile>> {
    let path = vault_path()?;
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if data.len() > 16 * 1024 {
        return Err(invalid("trusted profile too large"));
    }
    let clear = decrypt(&data)?;
    let profile: TrustedProfile = serde_json::from_slice(&clear)
        .map_err(io::Error::other)?;
    if !valid_profile(&profile) {
        return Err(invalid("trusted profile is invalid"));
    }
    Ok(Some(profile))
}

pub fn save(profile: &TrustedProfile) -> io::Result<()> {
    if !valid_profile(profile) {
        return Err(invalid("invalid trusted profile configuration"));
    }
    let clear = serde_json::to_vec(profile).map_err(io::Error::other)?;
    let encrypted = encrypt(&clear)?;
    let path = vault_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, encrypted)?;
    if let Err(error) = fs::rename(&temp, &path) {
        // On Windows, rename does not overwrite an existing destination.
        if error.kind() == io::ErrorKind::AlreadyExists {
            fs::remove_file(&path)?;
            fs::rename(temp, path)?;
        } else {
            return Err(error);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDeviceIdentity {
    device_id: String,
    signing_seed_hex: String,
}

fn identity_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is unset"))?;
    Ok(PathBuf::from(base).join("P2PRemote").join("device-identity.bin"))
}

/// Load or create the long-lived Ed25519 device identity. Existing unreadable
/// identity files fail closed; they are never silently replaced.
pub fn load_or_create_device_identity() -> io::Result<(String, [u8; 32])> {
    let path = identity_path()?;
    match fs::read(&path) {
        Ok(encrypted) => {
            if encrypted.len() > 4096 {
                return Err(invalid("device identity file too large"));
            }
            let clear = decrypt(&encrypted)?;
            let stored: StoredDeviceIdentity =
                serde_json::from_slice(&clear).map_err(io::Error::other)?;
            if stored.device_id.len() != 32
                || !stored.device_id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || stored.signing_seed_hex.len() != 64
                || !stored.signing_seed_hex.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("stored device identity is invalid"));
            }
            let seed = hex::decode(stored.signing_seed_hex)
                .map_err(|_| invalid("stored device identity seed is invalid"))?;
            let seed = <[u8; 32]>::try_from(seed)
                .map_err(|_| invalid("stored device identity seed has wrong length"))?;
            Ok((stored.device_id, seed))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            use rand::RngCore;
            let mut device_id = [0u8; 16];
            let mut seed = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut device_id);
            rand::rngs::OsRng.fill_bytes(&mut seed);
            let stored = StoredDeviceIdentity {
                device_id: hex::encode(device_id),
                signing_seed_hex: hex::encode(seed),
            };
            let clear = serde_json::to_vec(&stored).map_err(io::Error::other)?;
            let encrypted = encrypt(&clear)?;
            if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
            let temp = path.with_extension("tmp");
            fs::write(&temp, encrypted)?;
            if let Err(rename_error) = fs::rename(&temp, &path) {
                let _ = fs::remove_file(&temp);
                return Err(rename_error);
            }
            Ok((stored.device_id, seed))
        }
        Err(error) => Err(error),
    }
}

pub fn revoke() -> io::Result<()> {
    match fs::remove_file(vault_path()?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Per-user login start via HKCU Run. No admin rights, no stealth service.
/// Never place a secret on the Windows command line or in the Run entry.
pub fn set_login_start(enabled: bool) -> io::Result<()> {
    if !cfg!(windows) {
        return Ok(());
    }
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = Command::new("reg.exe");
    if enabled {
        let executable = std::env::current_exe()?;
        let quoted = format!("\"{}\" --autostart", executable.display());
        command.args(["add", key, "/v", "P2PRemote", "/t", "REG_SZ", "/d", &quoted, "/f"]);
    } else {
        command.args(["delete", key, "/v", "P2PRemote", "/f"]);
    }
    let output = command.output()?;
    if !output.status.success() {
        if !enabled && output.status.code() == Some(1) {
            return Ok(()); // no previous Run entry
        }
        return Err(io::Error::new(io::ErrorKind::Other, "cannot update per-user auto-start registry"));
    }
    Ok(())
}

#[cfg(windows)]
mod dpapi {
    use super::*;
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "Crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "Kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    fn transform(input: &[u8], protect: bool) -> io::Result<Vec<u8>> {
        if input.is_empty() || input.len() > 16 * 1024 {
            return Err(invalid("invalid encrypted profile length"));
        }
        let blob = DataBlob {
            cb_data: input.len() as u32,
            pb_data: input.as_ptr() as *mut u8,
        };
        let mut output = DataBlob { cb_data: 0, pb_data: ptr::null_mut() };
        let success = unsafe {
            if protect {
                CryptProtectData(&blob, ptr::null(), ptr::null(), ptr::null_mut(),
                    ptr::null(), 0x1, &mut output)
            } else {
                CryptUnprotectData(&blob, ptr::null_mut(), ptr::null(), ptr::null_mut(),
                    ptr::null(), 0x1, &mut output)
            }
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        if output.pb_data.is_null() || output.cb_data == 0 || output.cb_data > 16 * 1024 {
            if !output.pb_data.is_null() {
                unsafe { LocalFree(output.pb_data.cast()); }
            }
            return Err(invalid("DPAPI returned an invalid buffer"));
        }
        let result = unsafe {
            std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec()
        };
        if !protect {
            unsafe { ptr::write_bytes(output.pb_data, 0, output.cb_data as usize) };
        }
        unsafe { LocalFree(output.pb_data.cast()); }
        Ok(result)
    }

    pub fn encrypt(data: &[u8]) -> io::Result<Vec<u8>> { transform(data, true) }
    pub fn decrypt(data: &[u8]) -> io::Result<Vec<u8>> { transform(data, false) }
}

#[cfg(windows)]
use dpapi::{decrypt, encrypt};

#[cfg(not(windows))]
fn encrypt(_: &[u8]) -> io::Result<Vec<u8>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Windows DPAPI required"))
}
#[cfg(not(windows))]
fn decrypt(_: &[u8]) -> io::Result<Vec<u8>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Windows DPAPI required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_must_have_strong_credentials() {
        let profile = TrustedProfile {
            relay_url: "wss://relay.example.com".into(),
            relay_token: "x".repeat(64),
            session_hex: "a".repeat(64),
            launch_on_login: true,
            unattended_input: false,
        };
        assert!(valid_profile(&profile));
        assert!(!valid_profile(&TrustedProfile { relay_token: "short".into(), ..profile.clone() }));
        assert!(!valid_profile(&TrustedProfile { relay_url: "ws://evil.example.com".into(), ..profile.clone() }));
        assert!(!valid_profile(&TrustedProfile { session_hex: "q".repeat(64), ..profile }));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_with_current_windows_user() {
        let input = b"p2p test encrypted secret: do not store this plaintext";
        let sealed = encrypt(input).expect("encrypt current-user secret");
        assert_ne!(sealed, input);
        let recovered = decrypt(&sealed).expect("decrypt current-user secret");
        assert_eq!(recovered, input);
        let mut changed = sealed;
        let index = changed.len() / 2;
        changed[index] ^= 1;
        assert!(decrypt(&changed).is_err(), "damaged protected blob must not decrypt");
    }
}

//! Shared native core for the Flutter Windows client.
//! The legacy Iced binary and the Flutter FFI library compile the same engine modules.
mod host;
#[cfg(windows)]
mod host_pairing;
mod protocol;
mod pairing_crypto;
mod pairing_api;
mod reconnect;
mod local_settings;
mod remote_ops;
mod trusted_profile;
mod video;
#[cfg(windows)]
mod mf_encoder;
mod lan;
mod stun;
mod udp_punch;
mod udp_video;
mod viewer;
mod viewer_geometry;
mod tls;

#[cfg(windows)]
pub mod flutter_bridge;

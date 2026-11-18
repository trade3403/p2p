//! V0.6 video capture/encoding on a dedicated thread.
//! JPEG remains default; OpenH264 is opt-in software encoding. Frame queue
//! capacity is small to favor current frames over stale frames on slow links.
use crate::protocol;
use std::{
    sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub type Frame = (u8, Vec<u8>);

pub fn dimensions(source_w: u32, source_h: u32) -> (u32, u32) {
    if source_w < 2 || source_h < 2 { return (0, 0); }
    let scale = (1280.0_f64 / source_w as f64)
        .min(720.0_f64 / source_h as f64)
        .min(1.0);
    let w = (((source_w as f64 * scale).round() as u32).max(2).min(1280)) & !1;
    let h = (((source_h as f64 * scale).round() as u32).max(2).min(720)) & !1;
    (w.max(2), h.max(2))
}

#[cfg(windows)]
pub fn spawn_frames(
    monitor: Arc<AtomicUsize>,
    h264: Arc<AtomicBool>,
    target_fps: Arc<AtomicU32>,
    drops: Arc<AtomicU64>,
    udp_active: Arc<AtomicBool>,
    force_jpeg: Arc<AtomicBool>,
) -> mpsc::Receiver<Frame> {
    let (tx, rx) = mpsc::channel(2);
    thread::spawn(move || {
        let mut encoder: Option<openh264::encoder::Encoder> = None;
        let mut previous_h264 = false;
        let mut previous_monitor = usize::MAX;
        let mut frames_since_reset = 0u32;

        while !tx.is_closed() {
            if udp_active.load(Ordering::Relaxed) {
                // The encrypted UDP preview has its own low-res capture worker.
                thread::sleep(Duration::from_millis(120));
                continue;
            }
            let started = Instant::now();
            // Session-local JPEG override does not modify the owner's H.264 preference.
            let active_h264 = h264.load(Ordering::Relaxed)
                && !force_jpeg.load(Ordering::Relaxed);
            let selected = monitor.load(Ordering::Relaxed);
            if active_h264 != previous_h264
                || selected != previous_monitor
                || (active_h264 && frames_since_reset >= 30)
            {
                // New encoder emits fresh SPS/PPS and a decodable IDR. Repeated
                // every 30 frames so a newly attached Android viewer can recover.
                encoder = None;
                frames_since_reset = 0;
                previous_h264 = active_h264;
                previous_monitor = selected;
            }

            match capture(selected, active_h264, &mut encoder) {
                Ok(Some(frame)) => {
                    frames_since_reset += 1;
                    match tx.try_send(frame) {
                        Ok(()) => (),
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            drops.fetch_add(1, Ordering::Relaxed);
                            // Never keep a GOP with missing P frames. Restart so
                            // the next video access unit contains SPS/PPS + IDR.
                            encoder = None;
                            frames_since_reset = 0;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                Ok(None) => (),
                Err(_) => {
                    encoder = None;
                    frames_since_reset = 0;
                    thread::sleep(Duration::from_millis(200));
                }
            }

            let fps = if active_h264 {
                target_fps.load(Ordering::Relaxed).clamp(5, 20)
            } else {
                6
            };
            let budget = Duration::from_millis(1000 / fps as u64);
            let delay = budget.saturating_sub(started.elapsed());
            if !delay.is_zero() { thread::sleep(delay); }
        }
    });
    rx
}

#[cfg(not(windows))]
pub fn spawn_frames(
    _monitor: Arc<AtomicUsize>,
    _h264: Arc<AtomicBool>,
    _target_fps: Arc<AtomicU32>,
    _drops: Arc<AtomicU64>,
    _udp_active: Arc<AtomicBool>,
    _force_jpeg: Arc<AtomicBool>,
) -> mpsc::Receiver<Frame> {
    let (_tx, rx) = mpsc::channel(1);
    rx
}

#[cfg(windows)]
fn capture(
    screen_index: usize,
    h264: bool,
    encoder: &mut Option<openh264::encoder::Encoder>,
) -> anyhow::Result<Option<Frame>> {
    use anyhow::{anyhow, Context};
    use image::{imageops::FilterType, codecs::jpeg::JpegEncoder, DynamicImage, RgbaImage};
    use openh264::{
        encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode},
        formats::{RgbaSliceU8, YUVBuffer},
        OpenH264API,
    };
    let screen = screenshots::Screen::all()?.into_iter().nth(screen_index)
        .context("selected display not available")?;
    let pixels = screen.capture()?;
    let (raw_w, raw_h) = (pixels.width(), pixels.height());
    let (w, h) = dimensions(raw_w, raw_h);
    if w < 2 || h < 2 { return Err(anyhow!("display has invalid dimensions")); }
    let rgba = RgbaImage::from_raw(raw_w, raw_h, pixels.into_raw())
        .context("invalid capture buffer")?;
    let image = DynamicImage::ImageRgba8(rgba).resize_exact(w, h, FilterType::Triangle);
    if h264 {
        if encoder.is_none() {
            let options = EncoderConfig::new()
                .bitrate(BitRate::from_bps(2_000_000))
                .max_frame_rate(FrameRate::from_hz(20.0))
                .intra_frame_period(IntraFramePeriod::from_num_frames(30))
                .rate_control_mode(RateControlMode::Bitrate)
                .skip_frames(false);
            *encoder = Some(Encoder::with_api_config(OpenH264API::from_source(), options)?);
        }
        let rgba = image.to_rgba8();
        let source = RgbaSliceU8::new(rgba.as_raw(), (w as usize, h as usize));
        let yuv = YUVBuffer::from_rgb_source(source);
        let stream = encoder.as_mut().context("encoder unavailable")?.encode(&yuv)?;
        let bytes = stream.to_vec();
        if bytes.is_empty() { return Ok(None); }
        let mut output = Vec::with_capacity(bytes.len() + 4);
        output.extend_from_slice(&(w as u16).to_be_bytes());
        output.extend_from_slice(&(h as u16).to_be_bytes());
        output.extend_from_slice(&bytes); // one Annex-B H264 access unit
        if output.len() > 2 * 1024 * 1024 { return Err(anyhow!("encoded AVC frame too large")); }
        Ok(Some((protocol::AVC, output)))
    } else {
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 58).encode_image(&image)?;
        Ok(Some((protocol::FRAME, bytes)))
    }
}

/// Separate low-bandwidth preview used ONLY after an authenticated V0.10
/// UDP handshake. Never captures lock/UAC secure desktops.
#[cfg(windows)]
pub fn capture_udp_jpeg(index: usize) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context;
    use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, RgbaImage};
    let display = screenshots::Screen::all()?.into_iter().nth(index)
        .context("selected display missing for UDP preview")?;
    let captured = display.capture()?;
    let image = RgbaImage::from_raw(
        captured.width(), captured.height(), captured.into_raw()
    ).context("invalid screen capture bytes")?;
    let resized = DynamicImage::ImageRgba8(image)
        .resize(640, 360, FilterType::Triangle);
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 40).encode_image(&resized)?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolution_is_bounded_even_and_aspect_preserved_approximately() {
        for (w, h) in [(1920,1080), (3440,1440), (1080,1920), (640,480), (1,1)] {
            let (out_w, out_h) = dimensions(w,h);
            if w < 2 || h < 2 { assert_eq!((out_w,out_h),(0,0));continue; }
            assert!(out_w <= 1280 && out_h <= 720);
            assert_eq!(out_w % 2,0);
            assert_eq!(out_h % 2,0);
        }
        assert_eq!(dimensions(1920,1080), (1280,720));
    }

    #[cfg(windows)]
    #[test]
    fn software_h264_encoder_emits_annex_b_start_codes() {
        use openh264::{encoder::Encoder, formats::{RgbaSliceU8, YUVBuffer}};
        let rgb = vec![100u8; 64 * 64 * 4];
        let source = RgbaSliceU8::new(&rgb, (64, 64));
        let yuv = YUVBuffer::from_rgb_source(source);
        let mut encoder = Encoder::new().expect("openh264 encoder initialized");
        let first = encoder.encode(&yuv).expect("test frame encoded").to_vec();
        assert!(!first.is_empty());
        assert!(first.windows(3).any(|chunk| chunk == [0u8,0,1]));
    }
}

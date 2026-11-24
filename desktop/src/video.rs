//! V0.21 bounded desktop capture and negotiated AVC encoding on one worker.
//! DXGI is preferred for unrotated displays; compatibility capture is retained.
use crate::protocol;
use std::{
    sync::{atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub type Frame = (u8, Vec<u8>);

#[derive(Default)]
pub struct VideoStats {
    pub capture_backend: AtomicU32, // 0 unknown, 1 compatibility, 2 DXGI
    pub fresh_frames: AtomicU64,
    pub reused_frames: AtomicU64,
    pub resize_cache_hits: AtomicU64,
    pub yuv_cache_hits: AtomicU64,
    pub capture_total_us: AtomicU64,
    pub capture_samples: AtomicU64,
    pub encode_total_us: AtomicU64,
    pub encode_samples: AtomicU64,
    pub send_total_us: AtomicU64,
    pub send_samples: AtomicU64,
}
impl VideoStats {
    pub fn average(total:&AtomicU64,count:&AtomicU64)->u64 {
        total.load(Ordering::Relaxed).checked_div(count.load(Ordering::Relaxed)).unwrap_or(0)
    }
    pub fn record(total:&AtomicU64,count:&AtomicU64,start:Instant) {
        total.fetch_add(start.elapsed().as_micros().min(u64::MAX as u128) as u64,Ordering::Relaxed);
        count.fetch_add(1,Ordering::Relaxed);
    }
}

// Cached scaled pixels and software color conversion are only reusable when
// the SAME DXGI duplication reports no update. All fallback/error/display
// transitions invalidate the cache.
#[cfg(windows)]
#[derive(Default)]
struct PixelCache {
    geometry: Option<(i32,i32,u32,u32)>,
    rgba: Option<image::RgbaImage>,
    yuv: Option<openh264::formats::YUVBuffer>,
}
#[cfg(windows)]
impl PixelCache {
    fn clear(&mut self) {
        self.geometry=None;
        self.rgba=None;
        self.yuv=None;
    }
}

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
    bitrate: Arc<AtomicU32>,
    drops: Arc<AtomicU64>,
    udp_active: Arc<AtomicBool>,
    force_jpeg: Arc<AtomicBool>,
    actual_encoder: Arc<AtomicU32>,
    stats: Arc<VideoStats>,
) -> mpsc::Receiver<Frame> {
    let (tx, rx) = mpsc::channel(2);
    thread::spawn(move || {
        let mut encoder: Option<openh264::encoder::Encoder> = None;
        let mut hardware: Option<crate::mf_encoder::HardwareEncoder> = None;
        // No repeated expensive hardware probing on a host without a usable MFT.
        let mut hardware_failed = false;
        let mut gpu:Option<crate::mf_encoder::GpuEncoder>=None;
        let mut gpu_failed=false;
        let mut desktop_capture=crate::dxgi_capture::DesktopCapture::default();
        let mut pixels=PixelCache::default();
        let mut previous_h264 = false;
        let mut previous_monitor = usize::MAX;
        let mut frames_since_reset = 0u32;
        let mut previous_profile = (0u32,0u32);

        while !tx.is_closed() {
            if udp_active.load(Ordering::Relaxed) {
                gpu=None;
                desktop_capture.clear();
                pixels.clear();
                // The encrypted UDP preview has its own low-res capture worker.
                thread::sleep(Duration::from_millis(120));
                continue;
            }
            let started = Instant::now();
            // Session-local JPEG override does not modify the owner's H.264 preference.
            let active_h264 = h264.load(Ordering::Relaxed)
                && !force_jpeg.load(Ordering::Relaxed);
            let selected = monitor.load(Ordering::Relaxed);
            let profile = (bitrate.load(Ordering::Relaxed).clamp(400_000,4_000_000),
                           target_fps.load(Ordering::Relaxed).clamp(5,30));
            if active_h264 != previous_h264
                || selected != previous_monitor
                || (active_h264 && ((gpu.is_none() && hardware.is_none() && frames_since_reset >= profile.1) || previous_profile != profile))
            {
                // New encoder emits fresh SPS/PPS and a decodable IDR. Repeated
                // every 30 frames so a newly attached Android viewer can recover.
                encoder = None;
                hardware = None;
                gpu=None;
                if active_h264!=previous_h264 || selected!=previous_monitor {
                    desktop_capture.clear();
                    pixels.clear();
                    gpu_failed=false;
                }
                frames_since_reset = 0;
                previous_h264 = active_h264;
                previous_monitor = selected;
                previous_profile = profile;
            }

            match capture(selected, active_h264, profile, &mut encoder, &mut hardware,
                          &mut hardware_failed, &actual_encoder, &mut desktop_capture, &mut pixels, &mut gpu, &mut gpu_failed, &stats) {
                Ok(Some(frame)) => {
                    frames_since_reset += 1;
                    match tx.try_send(frame) {
                        Ok(()) => (),
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            drops.fetch_add(1, Ordering::Relaxed);
                            // Never keep a GOP with missing P frames. Restart so
                            // the next video access unit contains SPS/PPS + IDR.
                            encoder = None;
                            hardware = None;
                            gpu=None;
                            frames_since_reset = 0;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                Ok(None) => (),
                Err(_) => {
                    encoder = None;
                    hardware = None;
                    gpu=None;
                    desktop_capture.clear();
                    pixels.clear();
                    frames_since_reset = 0;
                    thread::sleep(Duration::from_millis(200));
                }
            }

            let fps = if active_h264 {
                target_fps.load(Ordering::Relaxed).clamp(5, 30)
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
    _bitrate: Arc<AtomicU32>,
    _drops: Arc<AtomicU64>,
    _udp_active: Arc<AtomicBool>,
    _force_jpeg: Arc<AtomicBool>,
    _actual_encoder: Arc<AtomicU32>,
    _stats: Arc<VideoStats>,
) -> mpsc::Receiver<Frame> {
    let (_tx, rx) = mpsc::channel(1);
    rx
}

#[cfg(windows)]
fn capture(
    screen_index: usize,
    h264: bool,
    profile: (u32,u32),
    encoder: &mut Option<openh264::encoder::Encoder>,
    hardware: &mut Option<crate::mf_encoder::HardwareEncoder>,
    hardware_failed: &mut bool,
    actual_encoder: &AtomicU32,
    desktop_capture: &mut crate::dxgi_capture::DesktopCapture,
    cache: &mut PixelCache,
    gpu: &mut Option<crate::mf_encoder::GpuEncoder>,
    gpu_failed: &mut bool,
    stats: &VideoStats,
) -> anyhow::Result<Option<Frame>> {
    use anyhow::{anyhow, Context};
    use image::{imageops::FilterType, codecs::jpeg::JpegEncoder, RgbaImage};
    use openh264::{
        encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode},
        formats::{RgbaSliceU8, YUVBuffer},
        OpenH264API,
    };
    use crate::dxgi_capture::{desktop_allowed,Read};
    let capture_start=Instant::now();
    if !desktop_allowed() {
        desktop_capture.clear();
        cache.clear();
        return Err(anyhow!("interactive desktop unavailable"));
    }
    let screen = screenshots::Screen::all()?.into_iter().nth(screen_index)
        .context("selected display not available")?;
    let info=&screen.display_info;
    let geometry=(info.x,info.y,info.width,info.height);
    if cache.geometry!=Some(geometry) {cache.clear();}
    use crate::mf_encoder::{GpuEncoder,GpuRead};
    // Never hold two desktop duplications for the same output.
    if h264 && !*gpu_failed {
        if gpu.as_ref().is_some_and(|g|!g.matches(geometry,profile)){*gpu=None;}
        if gpu.is_none() {
            desktop_capture.clear();cache.clear();*hardware=None;*encoder=None;
            *gpu=GpuEncoder::new(geometry,profile,dimensions(info.width,info.height));
            if gpu.is_none(){*gpu_failed=true;}
        }
        if let Some(g)=gpu.as_mut() {
            let (w,h)=g.size;
            match g.read() {
                GpuRead::Frame{bytes,fresh,capture_us,encode_us}=>{
                    let mut packet=Vec::with_capacity(bytes.len()+4);
                    packet.extend_from_slice(&(w as u16).to_be_bytes());
                    packet.extend_from_slice(&(h as u16).to_be_bytes());packet.extend_from_slice(&bytes);
                    stats.capture_backend.store(2,Ordering::Relaxed);
                    if fresh{stats.fresh_frames.fetch_add(1,Ordering::Relaxed);}
                    else{stats.reused_frames.fetch_add(1,Ordering::Relaxed);}
                    stats.capture_total_us.fetch_add(capture_us,Ordering::Relaxed);
                    stats.capture_samples.fetch_add(1,Ordering::Relaxed);
                    stats.encode_total_us.fetch_add(encode_us,Ordering::Relaxed);
                    stats.encode_samples.fetch_add(1,Ordering::Relaxed);
                    actual_encoder.store(3,Ordering::Relaxed);
                    return Ok(Some((protocol::AVC,packet)));
                }
                GpuRead::Idle=>return Ok(None),
                GpuRead::Blocked=>{*gpu=None;return Err(anyhow!("GPU desktop capture blocked"));}
                GpuRead::Failed=>{*gpu=None;*gpu_failed=true;}
            }
        }
    } else {*gpu=None;}

    // Legacy JPEG behavior is retained for protocol compatibility.
    let result=if h264 {desktop_capture.read(geometry)}
        else {desktop_capture.clear();Read::Fallback};
    let (fresh,backend,reused)=match result {
        Read::Fresh(img)=>(Some(img),2,false),
        Read::Reused if cache.geometry==Some(geometry) && cache.rgba.is_some()=>(None,2,true),
        // Never reuse a stale cached image after losing its provenance.
        Read::Reused=>{desktop_capture.clear();cache.clear();return Ok(None);}
        Read::Blocked=>{cache.clear();return Err(anyhow!("desktop capture blocked"));}
        Read::Fallback=>{
            cache.clear();
            if !desktop_allowed(){return Err(anyhow!("interactive desktop unavailable"));}
            let pixels=screen.capture()?;
            let (rw,rh)=(pixels.width(),pixels.height());
            (Some(RgbaImage::from_raw(rw,rh,pixels.into_raw()).context("invalid capture buffer")?),1,false)
        }
    };
    if !desktop_allowed(){
        desktop_capture.clear();
        cache.clear();
        return Err(anyhow!("desktop changed during capture"));
    }
    stats.capture_backend.store(backend,Ordering::Relaxed);
    if reused {stats.reused_frames.fetch_add(1,Ordering::Relaxed);}
    else {stats.fresh_frames.fetch_add(1,Ordering::Relaxed);}
    VideoStats::record(&stats.capture_total_us,&stats.capture_samples,capture_start);
    let encode_start=Instant::now();
    if let Some(rgba)=fresh {
        let (raw_w,raw_h)=rgba.dimensions();
        let (w,h)=dimensions(raw_w,raw_h);
        if w<2||h<2 {return Err(anyhow!("display has invalid dimensions"));}
        cache.rgba=Some(if (raw_w,raw_h)==(w,h) {rgba} else {
            image::imageops::resize(&rgba,w,h,FilterType::Triangle)
        });
        cache.yuv=None;
        cache.geometry=Some(geometry);
    } else {
        stats.resize_cache_hits.fetch_add(1,Ordering::Relaxed);
    }
    let image=cache.rgba.as_ref().context("scaled frame missing")?;
    let (w,h)=image.dimensions();
    if h264 {
        // Hardware is classified only after a successfully emitted, validated AVC unit.
        if hardware.as_ref().is_some_and(|e|e.dimensions()!=(w,h)) { *hardware=None; }
        if hardware.is_none() && !*hardware_failed {
            *hardware=crate::mf_encoder::HardwareEncoder::new(w,h,profile.1,profile.0);
            if hardware.is_none() { *hardware_failed=true; }
        }
        if let Some(hw)=hardware.as_mut() {
            if let Some(bytes)=hw.encode(image.as_raw(),reused) {
                let mut packet=Vec::with_capacity(bytes.len()+4);
                packet.extend_from_slice(&(w as u16).to_be_bytes());
                packet.extend_from_slice(&(h as u16).to_be_bytes());
                packet.extend_from_slice(&bytes);
                actual_encoder.store(2,Ordering::Relaxed);
                VideoStats::record(&stats.encode_total_us,&stats.encode_samples,encode_start);
                return Ok(Some((protocol::AVC,packet)));
            }
            // Fail closed to the validated software encoder with a fresh IDR.
            *hardware=None;
            *hardware_failed=true;
            *encoder=None;
        }
        if encoder.is_none() {
            let options = EncoderConfig::new()
                .bitrate(BitRate::from_bps(profile.0))
                .max_frame_rate(FrameRate::from_hz(profile.1 as f32))
                .intra_frame_period(IntraFramePeriod::from_num_frames(profile.1))
                .rate_control_mode(RateControlMode::Bitrate)
                .skip_frames(false);
            *encoder = Some(Encoder::with_api_config(OpenH264API::from_source(), options)?);
        }
        if reused && cache.yuv.is_some() {
            stats.yuv_cache_hits.fetch_add(1,Ordering::Relaxed);
        } else {
            let source=RgbaSliceU8::new(image.as_raw(),(w as usize,h as usize));
            cache.yuv=Some(YUVBuffer::from_rgb_source(source));
        }
        let yuv=cache.yuv.as_ref().context("YUV frame missing")?;
        let stream = encoder.as_mut().context("encoder unavailable")?.encode(yuv)?;
        let bytes = stream.to_vec();
        if bytes.is_empty() { return Ok(None); }
        let mut output = Vec::with_capacity(bytes.len() + 4);
        output.extend_from_slice(&(w as u16).to_be_bytes());
        output.extend_from_slice(&(h as u16).to_be_bytes());
        output.extend_from_slice(&bytes); // one Annex-B H264 access unit
        if output.len() > 2 * 1024 * 1024 { return Err(anyhow!("encoded AVC frame too large")); }
        actual_encoder.store(1,Ordering::Relaxed);
        VideoStats::record(&stats.encode_total_us,&stats.encode_samples,encode_start);
        Ok(Some((protocol::AVC, output)))
    } else {
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 58).encode_image(image)?;
        actual_encoder.store(0,Ordering::Relaxed);
        VideoStats::record(&stats.encode_total_us,&stats.encode_samples,encode_start);
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

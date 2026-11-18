//! Pure remote-screen geometry and monitor-selection helpers for the Windows viewer.
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Monitors {
    pub count: usize,
    pub selected: usize,
}
impl Monitors {
    pub fn parse(json: &str) -> Option<Self> {
        let metadata: Self = serde_json::from_str(json).ok()?;
        (metadata.count > 0 && metadata.count <= 16 && metadata.selected < metadata.count).then_some(metadata)
    }
    pub fn previous(self) -> usize { (self.selected + self.count - 1) % self.count }
    pub fn next(self) -> usize { (self.selected + 1) % self.count }
}

/// Compute the visible viewer surface. All UI drawing and input events must
/// use this one function, including after a real OS fullscreen resize.
pub fn viewer_dimensions(
    fullscreen: bool,
    window_size: (f32, f32),
    windowed: (f32, f32),
) -> (f32, f32) {
    if !fullscreen { return windowed; }
    let (window_w, window_h) = window_size;
    if !window_w.is_finite() || !window_h.is_finite() { return windowed; }
    let available_w = (window_w - 40.0).max(320.0);
    let available_h = (window_h - 152.0).max(200.0);
    let width = available_w.min(available_h * 16.0 / 9.0);
    (width, width * 9.0 / 16.0)
}

/// Map a pointer in a fixed viewer surface into normalized coordinates in the
/// remote image, respecting black bars caused by aspect-ratio fitting.
/// Returns None for clicks on empty margins rather than clicking the remote desktop.
pub fn normalized_pointer(
    x: f32, y: f32, surface_w: f32, surface_h: f32, image_w: u32, image_h: u32,
) -> Option<(f32, f32)> {
    if !x.is_finite() || !y.is_finite() || surface_w <= 0.0 || surface_h <= 0.0
        || image_w == 0 || image_h == 0 || !surface_w.is_finite() || !surface_h.is_finite()
    { return None; }
    let scale = (surface_w / image_w as f32).min(surface_h / image_h as f32);
    let shown_w = image_w as f32 * scale;
    let shown_h = image_h as f32 * scale;
    let left = (surface_w - shown_w) / 2.0;
    let top = (surface_h - shown_h) / 2.0;
    if x < left || y < top || x > left + shown_w || y > top + shown_h { return None; }
    Some((((x - left) / shown_w).clamp(0.0,1.0), ((y - top) / shown_h).clamp(0.0,1.0)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn letterbox_mapping() {
        assert_eq!(normalized_pointer(400.0,225.0,800.0,450.0,1920,1080),Some((0.5,0.5)));
        assert_eq!(normalized_pointer(0.0,225.0,800.0,450.0,1024,768),None);
        assert_eq!(normalized_pointer(400.0,225.0,800.0,450.0,1024,768),Some((0.5,0.5)));
        assert_eq!(normalized_pointer(-1.0,0.0,800.0,450.0,1920,1080),None);
        assert_eq!(normalized_pointer(f32::NAN,0.0,800.0,450.0,1920,1080),None);
    }
    #[test]
    fn fullscreen_surface_and_pointer_stay_in_sync() {
        let expected = (700.0, 394.0);
        assert_eq!(viewer_dimensions(false, (1920.0,1080.0), expected), expected);
        for screen in [(1366.0, 768.0), (1920.0, 1080.0), (2560.0, 1440.0)] {
            let (width, height) = viewer_dimensions(true, screen, expected);
            assert!(width > 0.0 && height > 0.0);
            assert!(width <= screen.0 && height < screen.1);
            let middle = normalized_pointer(
                width/2.0, height/2.0, width, height, 1920, 1080
            ).unwrap();
            assert!((middle.0-0.5).abs() < 0.0001);
            assert!((middle.1-0.5).abs() < 0.0001);
            // Non-widescreen remote content must not receive clicks on black bars.
            assert_eq!(normalized_pointer(0.0, height/2.0, width, height, 1024,768), None);
        }
        assert_eq!(
            viewer_dimensions(true, (f32::NAN, 600.0), expected), expected
        );
    }
    #[test]
    fn v0121_zoom_presets_and_non_widescreen_hosts() {
        for &(surface_w, surface_h) in &[
            (560.0, 315.0),
            (700.0, 394.0),
            (840.0, 473.0),
        ] {
            let center = normalized_pointer(
                surface_w / 2.0, surface_h / 2.0,
                surface_w, surface_h, 1920, 1080,
            ).expect("center must land inside image");
            assert!((center.0 - 0.5).abs() < 0.0001);
            assert!((center.1 - 0.5).abs() < 0.0001);
            assert_eq!(normalized_pointer(
                0.0, surface_h / 2.0, surface_w, surface_h, 1024, 768
            ), None, "pillarbox clicks must not reach 4:3 remote desktops");
        }
    }
    #[test]
    fn monitor_bounds_and_wrap() {
        let m=Monitors::parse(r#"{"count":3,"selected":2}"#).unwrap();
        assert_eq!(m.next(),0);
        assert_eq!(m.previous(),1);
        assert!(Monitors::parse(r#"{"count":0,"selected":0}"#).is_none());
        assert!(Monitors::parse(r#"{"count":2,"selected":2}"#).is_none());
        assert!(Monitors::parse(r#"{"count":99,"selected":0}"#).is_none());
    }
}

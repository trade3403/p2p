//! Capture a persistent DXGI output on the same worker thread.
use std::{ffi::c_void,ptr::NonNull,time::{Duration,Instant}};
use image::RgbaImage;
extern "C" {
    fn p2p_capture_desktop_allowed()->i32;
    fn p2p_dxgi_create(x:i32,y:i32,w:u32,h:u32)->*mut c_void;
    fn p2p_dxgi_read(handle:*mut c_void,rgba:*mut u8,n:usize)->i32;
    fn p2p_dxgi_destroy(handle:*mut c_void);
}
pub fn desktop_allowed()->bool { unsafe {p2p_capture_desktop_allowed()==1} }
type Geometry=(i32,i32,u32,u32);
struct Output {
    handle:NonNull<c_void>,
    rgba:RgbaImage,
    ready:bool,
}
impl Output {
    fn new((x,y,w,h):Geometry)->Option<Self>{
        if w<2||h<2||w>8192||h>8192||u64::from(w)*u64::from(h)>34_000_000{return None;}
        let handle=NonNull::new(unsafe{p2p_dxgi_create(x,y,w,h)})?;
        Some(Self{handle,rgba:RgbaImage::new(w,h),ready:false})
    }
}
impl Drop for Output {
    fn drop(&mut self){unsafe{p2p_dxgi_destroy(self.handle.as_ptr())}}
}
pub enum Read { Fresh, Reused, Fallback, Blocked }
#[derive(Default)]
pub struct DesktopCapture {
    geometry:Option<Geometry>,
    output:Option<Output>,
    retry_after:Option<Instant>,
}
impl DesktopCapture {
    pub fn clear(&mut self){self.output=None;self.geometry=None;self.retry_after=None;}
    // Borrow the persistent CPU readback buffer only after successful read.
    // No full-resolution clone; the worker never mutates it during encoding.
    pub fn image(&self)->Option<&RgbaImage> {
        self.output.as_ref().filter(|o|o.ready).map(|o|&o.rgba)
    }
    pub fn read(&mut self,g:Geometry)->Read {
        if !desktop_allowed(){self.clear();return Read::Blocked;}
        if self.geometry!=Some(g){self.clear();self.geometry=Some(g);}
        if self.output.is_none() {
            if self.retry_after.is_some_and(|t|Instant::now()<t){return Read::Fallback;}
            self.output=Output::new(g);
            if self.output.is_none(){
                self.retry_after=Some(Instant::now()+Duration::from_secs(5));
                return if desktop_allowed(){Read::Fallback}else{Read::Blocked};
            }
        }
        let output=self.output.as_mut().unwrap();
        let code=unsafe{p2p_dxgi_read(output.handle.as_ptr(),output.rgba.as_mut_ptr(),output.rgba.len())};
        // A timeout on the same valid duplication is not a new image.
        // Do not clone the full-resolution CPU buffer on idle frames.
        if code==0 && output.ready { return Read::Reused; }
        if code==1 {
            output.ready=true;
            return Read::Fresh;
        }
        // No stale pixel access after any unsuccessful non-reuse read.
        output.ready=false;
        if code == -2 {self.clear();return Read::Blocked;}
        if code != 0 {
            self.output=None;
            self.retry_after=Some(Instant::now()+Duration::from_secs(2));
        }
        if desktop_allowed(){Read::Fallback}else{self.clear();Read::Blocked}
    }
}

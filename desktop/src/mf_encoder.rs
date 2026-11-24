//! Media Foundation hardware-only encoder. Never marks enumeration alone as success.
use std::{ffi::c_void, ptr::NonNull};
extern "C" {
    fn p2p_mf_create(w:u32,h:u32,fps:u32,bitrate:u32)->*mut c_void;
    fn p2p_mf_encode(handle:*mut c_void,nv12:*const u8,n:usize,out:*mut u8,cap:usize,written:*mut usize)->i32;
    fn p2p_mf_destroy(handle:*mut c_void);
}
pub struct HardwareEncoder {
    handle: NonNull<c_void>,
    nv12: Vec<u8>,
    output: Vec<u8>,
    w: u32,
    h: u32,
    nv12_valid: bool,
}
impl HardwareEncoder {
    pub fn new(w:u32,h:u32,fps:u32,bitrate:u32)->Option<Self> {
        let handle=NonNull::new(unsafe {p2p_mf_create(w,h,fps,bitrate)})?;
        Some(Self{handle,nv12:vec![0;w as usize*h as usize*3/2],
            output:vec![0;2*1024*1024-4],w,h,nv12_valid:false})
    }
    pub fn dimensions(&self)->(u32,u32){(self.w,self.h)}
    pub fn encode(&mut self,rgba:&[u8],reuse:bool)->Option<Vec<u8>> {
        if !reuse || !self.nv12_valid {
            self.nv12_valid=false;
            rgba_to_nv12(rgba,self.w as usize,self.h as usize,&mut self.nv12)?;
            self.nv12_valid=true;
        }
        let mut n=0;
        let result=unsafe {p2p_mf_encode(self.handle.as_ptr(),self.nv12.as_ptr(),self.nv12.len(),
            self.output.as_mut_ptr(),self.output.len(),&mut n)};
        if result!=0 || n==0 || n>self.output.len(){return None;}
        Some(self.output[..n].to_vec())
    }
}
impl Drop for HardwareEncoder {
    fn drop(&mut self){unsafe {p2p_mf_destroy(self.handle.as_ptr())}}
}
fn rgba_to_nv12(rgba:&[u8],w:usize,h:usize,out:&mut[u8])->Option<()> {
    if w<2||h<2||w%2!=0||h%2!=0||rgba.len()!=w*h*4||out.len()!=w*h*3/2{return None;}
    let (y,uv)=out.split_at_mut(w*h);
    for row in (0..h).step_by(2) {
        for col in (0..w).step_by(2) {
            let (mut sr,mut sg,mut sb)=(0i32,0i32,0i32);
            for dy in 0..2 {for dx in 0..2 {
                let i=(row+dy)*w+col+dx;
                let (r,g,b)=(rgba[i*4] as i32,rgba[i*4+1] as i32,rgba[i*4+2] as i32);
                y[i]=(((66*r+129*g+25*b+128)>>8)+16).clamp(16,235) as u8;
                sr+=r;sg+=g;sb+=b;
            }}
            let (r,g,b)=((sr+2)/4,(sg+2)/4,(sb+2)/4);
            let i=row/2*w+col;
            uv[i]=(((-38*r-74*g+112*b+128)>>8)+128).clamp(16,240) as u8;
            uv[i+1]=(((112*r-94*g-18*b+128)>>8)+128).clamp(16,240) as u8;
        }
    }
    Some(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn nv12_layout_and_bounds() {
        let mut out=[0;6];
        assert_eq!(rgba_to_nv12(&[0;16],2,2,&mut out),Some(()));
        assert_eq!(out,[16,16,16,16,128,128]);
        assert_eq!(rgba_to_nv12(&[255;16],2,2,&mut out),Some(()));
        assert_eq!(out,[235,235,235,235,128,128]);
        assert_eq!(rgba_to_nv12(&[0;15],2,2,&mut out),None);
    }
}

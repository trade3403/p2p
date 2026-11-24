// Persistent D3D11 Desktop Duplication. CPU readback is explicit; this is
// not a GPU zero-copy encoder. Only capture the normal interactive desktop.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d3d11.h>
#include <d3d10.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#include <cstdint>
#include <memory>
#include <cwchar>
using Microsoft::WRL::ComPtr;
namespace {
bool normal_desktop(HDESK d) {
    wchar_t name[64]{};
    DWORD needed=0;
    return d && GetUserObjectInformationW(d,UOI_NAME,name,sizeof(name),&needed)
        && _wcsicmp(name,L"Default")==0;
}
bool interactive() {
    HDESK d=OpenInputDesktop(0,FALSE,DESKTOP_READOBJECTS);
    if(!d) return false;
    bool ok=normal_desktop(d) && normal_desktop(GetThreadDesktop(GetCurrentThreadId()));
    CloseDesktop(d);
    return ok;
}
struct Capture {
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
    ComPtr<IDXGIOutputDuplication> duplication;
    ComPtr<ID3D11Texture2D> staging;
    UINT w=0,h=0;
    ComPtr<ID3D11VideoDevice> vd;
    ComPtr<ID3D11VideoContext> vc;
    ComPtr<ID3D11VideoProcessorEnumerator> ve;
    ComPtr<ID3D11VideoProcessor> vp;
    ComPtr<ID3D11Texture2D> nv12;
    UINT ow=0,oh=0;
};
struct FrameGuard {
    IDXGIOutputDuplication* duplication;
    ~FrameGuard(){ if(duplication) duplication->ReleaseFrame(); }
};
struct MapGuard {
    ID3D11DeviceContext* context;
    ID3D11Texture2D* texture;
    ~MapGuard(){ context->Unmap(texture,0); }
};
}
extern "C" int p2p_capture_desktop_allowed() noexcept { return interactive()?1:0; }
extern "C" void* p2p_dxgi_create(int32_t x,int32_t y,uint32_t w,uint32_t h) noexcept {
    try {
        if(!interactive() || w<2 || h<2 || w>8192 || h>8192 || uint64_t(w)*h>34000000) return nullptr;
        ComPtr<IDXGIFactory1> factory;
        if(FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) return nullptr;
        for(UINT i=0;i<32;++i) {
            ComPtr<IDXGIAdapter1> adapter;
            if(FAILED(factory->EnumAdapters1(i,&adapter))) break;
            for(UINT j=0;j<32;++j) {
                ComPtr<IDXGIOutput> out;
                if(FAILED(adapter->EnumOutputs(j,&out))) break;
                DXGI_OUTPUT_DESC d{};
                if(FAILED(out->GetDesc(&d)) || !d.AttachedToDesktop) continue;
                const RECT& r=d.DesktopCoordinates;
                // Match physical coordinates used by input mapping; don't assume
                // adapter/output enumeration order matches screenshots::Screen.
                if(r.left!=x || r.top!=y || r.right-r.left!=LONG(w) || r.bottom-r.top!=LONG(h)) continue;
                // Rotated outputs require separate coordinate transforms; use legacy capture.
                if(d.Rotation!=DXGI_MODE_ROTATION_IDENTITY) return nullptr;
                auto s=std::make_unique<Capture>();
                if(FAILED(D3D11CreateDevice(adapter.Get(),D3D_DRIVER_TYPE_UNKNOWN,nullptr,
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,nullptr,0,D3D11_SDK_VERSION,
                    &s->device,nullptr,&s->context))) return nullptr;
                ComPtr<IDXGIOutput1> output;
                if(FAILED(out.As(&output)) || FAILED(output->DuplicateOutput(s->device.Get(),&s->duplication))) return nullptr;
                DXGI_OUTDUPL_DESC dd{};
                s->duplication->GetDesc(&dd);
                if(dd.ModeDesc.Width!=w || dd.ModeDesc.Height!=h ||
                   dd.ModeDesc.Format!=DXGI_FORMAT_B8G8R8A8_UNORM) return nullptr;
                D3D11_TEXTURE2D_DESC td{};
                td.Width=w; td.Height=h; td.MipLevels=1; td.ArraySize=1;
                td.Format=DXGI_FORMAT_B8G8R8A8_UNORM; td.SampleDesc.Count=1;
                td.Usage=D3D11_USAGE_STAGING; td.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
                if(FAILED(s->device->CreateTexture2D(&td,nullptr,&s->staging))) return nullptr;
                s->w=w;s->h=h;
                return s.release();
            }
        }
        return nullptr;
    } catch(...) { return nullptr; }
}
// 1 = acquired fresh desktop surface, 0 = no update, -1 = recreate/fallback,
// -2 = desktop/protected-content restriction (do not send cached or fallback image).
extern "C" int p2p_dxgi_read(void* handle,uint8_t* rgba,size_t n) noexcept {
    try {
        if(!handle || !rgba || !interactive()) return -2;
        auto& s=*static_cast<Capture*>(handle);
        if(n!=size_t(s.w)*s.h*4) return -1;
        DXGI_OUTDUPL_FRAME_INFO info{};
        ComPtr<IDXGIResource> resource;
        HRESULT hr=s.duplication->AcquireNextFrame(4,&info,&resource);
        if(hr==DXGI_ERROR_WAIT_TIMEOUT) return interactive()?0:-2;
        if(FAILED(hr)) return interactive()?-1:-2;
        FrameGuard frame{s.duplication.Get()};
        if(info.ProtectedContentMaskedOut) return -2;
        ComPtr<ID3D11Texture2D> texture;
        if(FAILED(resource.As(&texture))) return -1;
        D3D11_TEXTURE2D_DESC td{};texture->GetDesc(&td);
        if(td.Width!=s.w || td.Height!=s.h || td.Format!=DXGI_FORMAT_B8G8R8A8_UNORM) return -1;
        s.context->CopyResource(s.staging.Get(),texture.Get());
        D3D11_MAPPED_SUBRESOURCE mapped{};
        if(FAILED(s.context->Map(s.staging.Get(),0,D3D11_MAP_READ,0,&mapped))) return -1;
        MapGuard map{s.context.Get(),s.staging.Get()};
        if(!mapped.pData || mapped.RowPitch<size_t(s.w)*4) return -1;
        for(UINT y=0;y<s.h;++y) {
            auto* src=static_cast<const uint8_t*>(mapped.pData)+size_t(y)*mapped.RowPitch;
            auto* dst=rgba+size_t(y)*s.w*4;
            for(UINT x=0;x<s.w;++x) {
                dst[x*4]=src[x*4+2]; dst[x*4+1]=src[x*4+1];
                dst[x*4+2]=src[x*4]; dst[x*4+3]=255;
            }
        }
        return interactive()?1:-2;
    } catch(...) { return -1; }
}
// GPU textures remain on the capture device. Device/texture pointers are borrowed.
extern "C" void* p2p_dxgi_gpu_device(void* handle,uint32_t w,uint32_t h,uint32_t fps) noexcept {
    try {
        if(!handle || !interactive() || w<2 || h<2 || (w&1) || (h&1)) return nullptr;
        auto& s=*static_cast<Capture*>(handle);
        ComPtr<IDXGIDevice> gd;ComPtr<IDXGIAdapter> a;ComPtr<IDXGIAdapter1> a1;
        DXGI_ADAPTER_DESC1 desc{};
        if(FAILED(s.device.As(&gd)) || FAILED(gd->GetAdapter(&a)) ||
           FAILED(a.As(&a1)) || FAILED(a1->GetDesc1(&desc)) || (desc.Flags&DXGI_ADAPTER_FLAG_SOFTWARE)) return nullptr;
        ComPtr<ID3D10Multithread> mt;
        if(FAILED(s.context.As(&mt))) return nullptr;
        mt->SetMultithreadProtected(TRUE);
        if(FAILED(s.device.As(&s.vd)) || FAILED(s.context.As(&s.vc))) return nullptr;
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC cd{};
        cd.InputFrameFormat=D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
        cd.InputFrameRate={fps,1};cd.InputWidth=s.w;cd.InputHeight=s.h;
        cd.OutputFrameRate={fps,1};cd.OutputWidth=w;cd.OutputHeight=h;
        cd.Usage=D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;
        if(FAILED(s.vd->CreateVideoProcessorEnumerator(&cd,&s.ve))) return nullptr;
        UINT in=0,out=0;
        if(FAILED(s.ve->CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM,&in)) ||
           FAILED(s.ve->CheckVideoProcessorFormat(DXGI_FORMAT_NV12,&out)) ||
           !(in&D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT) || !(out&D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT) ||
           FAILED(s.vd->CreateVideoProcessor(s.ve.Get(),0,&s.vp))) return nullptr;
        s.ow=w;s.oh=h;
        RECT src{0,0,LONG(s.w),LONG(s.h)},dst{0,0,LONG(w),LONG(h)};
        s.vc->VideoProcessorSetStreamFrameFormat(s.vp.Get(),0,D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
        s.vc->VideoProcessorSetStreamSourceRect(s.vp.Get(),0,TRUE,&src);
        s.vc->VideoProcessorSetStreamDestRect(s.vp.Get(),0,TRUE,&dst);
        s.vc->VideoProcessorSetOutputTargetRect(s.vp.Get(),TRUE,&dst);
        s.vc->VideoProcessorSetStreamAutoProcessingMode(s.vp.Get(),0,FALSE);
        return s.device.Get();
    } catch(...) {return nullptr;}
}
// 1 fresh texture, 0 cached immutable texture, 2 waiting for first frame,
// -1 unavailable, -2 restricted desktop. No CPU Map or readback.
extern "C" int p2p_dxgi_gpu_read(void* handle,void** texture) noexcept {
    try {
        if(!handle || !texture || !interactive()) return -2;
        *texture=nullptr;
        auto& s=*static_cast<Capture*>(handle);
        if(!s.vp) return -1;
        DXGI_OUTDUPL_FRAME_INFO info{};ComPtr<IDXGIResource> resource;
        HRESULT hr=s.duplication->AcquireNextFrame(4,&info,&resource);
        if(hr==DXGI_ERROR_WAIT_TIMEOUT) {
            if(!interactive()) return -2;
            *texture=s.nv12.Get();return *texture?0:2;
        }
        if(FAILED(hr)) return interactive()?-1:-2;
        FrameGuard guard{s.duplication.Get()};
        if(info.ProtectedContentMaskedOut) return -2;
        ComPtr<ID3D11Texture2D> source;
        if(FAILED(resource.As(&source))) return -1;
        D3D11_TEXTURE2D_DESC sd{};source->GetDesc(&sd);
        if(sd.Width!=s.w || sd.Height!=s.h || sd.Format!=DXGI_FORMAT_B8G8R8A8_UNORM) return -1;
        // GPU copy before ReleaseFrame; VP input views may reject duplication textures.
        sd.Usage=D3D11_USAGE_DEFAULT;sd.CPUAccessFlags=0;sd.MiscFlags=0;sd.BindFlags=D3D11_BIND_SHADER_RESOURCE;
        ComPtr<ID3D11Texture2D> bgra;
        if(FAILED(s.device->CreateTexture2D(&sd,nullptr,&bgra))) return -1;
        s.context->CopyResource(bgra.Get(),source.Get());
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC iv{};iv.ViewDimension=D3D11_VPIV_DIMENSION_TEXTURE2D;
        ComPtr<ID3D11VideoProcessorInputView> input;
        if(FAILED(s.vd->CreateVideoProcessorInputView(bgra.Get(),s.ve.Get(),&iv,&input))) return -1;
        D3D11_TEXTURE2D_DESC td{};
        td.Width=s.ow;td.Height=s.oh;td.MipLevels=1;td.ArraySize=1;td.SampleDesc.Count=1;
        td.Format=DXGI_FORMAT_NV12;td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_RENDER_TARGET;
        // Fresh immutable surface: do not overwrite a texture retained by MFT.
        ComPtr<ID3D11Texture2D> nv;
        if(FAILED(s.device->CreateTexture2D(&td,nullptr,&nv))) return -1;
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC ov{};ov.ViewDimension=D3D11_VPOV_DIMENSION_TEXTURE2D;
        ComPtr<ID3D11VideoProcessorOutputView> output;
        if(FAILED(s.vd->CreateVideoProcessorOutputView(nv.Get(),s.ve.Get(),&ov,&output))) return -1;
        D3D11_VIDEO_PROCESSOR_STREAM stream{};stream.Enable=TRUE;stream.pInputSurface=input.Get();
        if(FAILED(s.vc->VideoProcessorBlt(s.vp.Get(),output.Get(),0,1,&stream))) return -1;
        if(!interactive()) return -2;
        s.nv12=nv;*texture=s.nv12.Get();return 1;
    } catch(...) {return -1;}
}
extern "C" void p2p_dxgi_destroy(void* handle) noexcept { delete static_cast<Capture*>(handle); }

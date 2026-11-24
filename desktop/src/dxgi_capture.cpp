// Persistent D3D11 Desktop Duplication. CPU readback is explicit; this is
// not a GPU zero-copy encoder. Only capture the normal interactive desktop.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d3d11.h>
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
extern "C" void p2p_dxgi_destroy(void* handle) noexcept { delete static_cast<Capture*>(handle); }

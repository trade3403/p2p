// Windows hardware-only AVC MFT. Called on one dedicated capture thread.
// CPU NV12 staging is deliberate; GPU/DXGI zero-copy is a separate milestone.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <mfapi.h>
#include <d3d11.h>
#include <mfidl.h>
#include <mferror.h>
#include <mftransform.h>
// ICodecAPI is declared by the SDK's DirectShow interface header;
// codecapi.h supplies the codec property GUIDs, not the COM interface.
#include <dshow.h>
#include <codecapi.h>
#include <wrl/client.h>
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <memory>
#include <vector>
using Microsoft::WRL::ComPtr;
namespace {
struct Session {
    bool com=false, mf=false, need=false, started=false;
    ComPtr<IMFTransform> transform;
    ComPtr<IMFMediaEventGenerator> events;
    ComPtr<ICodecAPI> codec;
    ComPtr<IMFDXGIDeviceManager> manager;
    DWORD in=0,out=0;
    UINT32 w=0,h=0,fps=0;
    LONGLONG pts=0;
    unsigned frame=0;
    std::vector<uint8_t> headers;
    // Private compressed bytes, never passed to the transform as a sample.
    // Reuse capacity only within this session.
    std::vector<uint8_t> encoded;
    ~Session() {
        if(transform && started) {
            transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM,0);
            transform->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH,0);
            transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING,0);
        }
        codec.Reset(); events.Reset(); transform.Reset(); manager.Reset();
        if(mf) MFShutdown();
        if(com) CoUninitialize();
    }
};
bool common(IMFMediaType* t, REFGUID subtype,UINT32 w,UINT32 h,UINT32 fps) {
    return SUCCEEDED(t->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video))
        && SUCCEEDED(t->SetGUID(MF_MT_SUBTYPE,subtype))
        && SUCCEEDED(MFSetAttributeSize(t,MF_MT_FRAME_SIZE,w,h))
        && SUCCEEDED(MFSetAttributeRatio(t,MF_MT_FRAME_RATE,fps,1))
        && SUCCEEDED(MFSetAttributeRatio(t,MF_MT_PIXEL_ASPECT_RATIO,1,1))
        && SUCCEEDED(t->SetUINT32(MF_MT_INTERLACE_MODE,MFVideoInterlace_Progressive));
}
void set_uint(ICodecAPI* api,const GUID& id,ULONG n) {
    if(!api) return;
    VARIANT value; VariantInit(&value); value.vt=VT_UI4; value.ulVal=n;
    api->SetValue(&id,&value);
}
void set_bool(ICodecAPI* api,const GUID& id,bool b) {
    if(!api) return;
    VARIANT value; VariantInit(&value); value.vt=VT_BOOL; value.boolVal=b?VARIANT_TRUE:VARIANT_FALSE;
    api->SetValue(&id,&value);
}
bool start(Session& s,IMFActivate* activation,UINT32 bitrate) {
    if(FAILED(activation->ActivateObject(IID_PPV_ARGS(&s.transform)))) return false;
    ComPtr<IMFAttributes> attrs;
    if(FAILED(s.transform->GetAttributes(&attrs))) return false;
    UINT32 asynchronous=0;
    if(FAILED(attrs->GetUINT32(MF_TRANSFORM_ASYNC,&asynchronous)) || !asynchronous
       || FAILED(attrs->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK,TRUE))) return false;
    attrs->SetUINT32(MF_LOW_LATENCY,TRUE);
    if(s.manager) {
        UINT32 aware=0;
        if(FAILED(attrs->GetUINT32(MF_SA_D3D11_AWARE,&aware)) || !aware ||
           FAILED(s.transform->ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER,
               reinterpret_cast<ULONG_PTR>(s.manager.Get())))) return false;
    }
    if(FAILED(s.transform.As(&s.events))) return false;
    DWORD ni=0,no=0;
    if(FAILED(s.transform->GetStreamCount(&ni,&no)) || ni!=1 || no!=1) return false;
    HRESULT ids=s.transform->GetStreamIDs(1,&s.in,1,&s.out);
    if(FAILED(ids) && ids!=E_NOTIMPL) return false;
    s.transform.As(&s.codec);
    set_bool(s.codec.Get(),CODECAPI_AVLowLatencyMode,true);
    set_uint(s.codec.Get(),CODECAPI_AVEncCommonRateControlMode,eAVEncCommonRateControlMode_CBR);
    set_uint(s.codec.Get(),CODECAPI_AVEncCommonMeanBitRate,bitrate);
    set_uint(s.codec.Get(),CODECAPI_AVEncMPVGOPSize,s.fps);
    set_uint(s.codec.Get(),CODECAPI_AVEncMPVDefaultBPictureCount,0);
    ComPtr<IMFMediaType> output,input;
    if(FAILED(MFCreateMediaType(&output)) || !common(output.Get(),MFVideoFormat_H264,s.w,s.h,s.fps)
       || FAILED(output->SetUINT32(MF_MT_AVG_BITRATE,bitrate))
       || FAILED(output->SetUINT32(MF_MT_MPEG2_PROFILE,eAVEncH264VProfile_Base))
       || FAILED(s.transform->SetOutputType(s.out,output.Get(),0))) return false;
    if(FAILED(MFCreateMediaType(&input)) || !common(input.Get(),MFVideoFormat_NV12,s.w,s.h,s.fps)
       || FAILED(input->SetUINT32(MF_MT_DEFAULT_STRIDE,s.w))
       || FAILED(s.transform->SetInputType(s.in,input.Get(),0))) return false;
    if(FAILED(s.transform->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,0))
       || FAILED(s.transform->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,0))) return false;
    s.started=true;
    return true;
}
size_t prefix(const uint8_t* p,size_t n,size_t i) {
    if(i+4<=n && p[i]==0 && p[i+1]==0 && p[i+2]==0 && p[i+3]==1) return 4;
    if(i+3<=n && p[i]==0 && p[i+1]==0 && p[i+2]==1) return 3;
    return 0;
}
// MF AVC normally returns Annex B. Accept only fully parsed, bounded NAL units.
bool annex(const uint8_t* p,size_t n,std::vector<uint8_t>& out) {
    out.clear();
    if(!p || n<5 || n>2*1024*1024) return false;
    if(prefix(p,n,0)) { out.assign(p,p+n); return true; }
    for(size_t i=0;i<n;) {
        if(n-i<4) return false;
        size_t k=(size_t(p[i])<<24)|(size_t(p[i+1])<<16)|(size_t(p[i+2])<<8)|p[i+3]; i+=4;
        if(!k || k>n-i || out.size()+k+4>2*1024*1024) return false;
        out.insert(out.end(),{0,0,0,1}); out.insert(out.end(),p+i,p+i+k); i+=k;
    }
    return !out.empty();
}
struct Nals { bool idr=false,sps=false,pps=false; std::vector<uint8_t> params; };
Nals scan(const std::vector<uint8_t>& v) {
    Nals r;
    for(size_t i=0;i<v.size();) {
        size_t k=prefix(v.data(),v.size(),i);
        if(!k || i+k>=v.size()) { ++i; continue; }
        size_t end=i+k+1;
        while(end<v.size() && !prefix(v.data(),v.size(),end)) ++end;
        unsigned t=v[i+k]&31;
        r.idr|=t==5; r.sps|=t==7; r.pps|=t==8;
        if(t==7 || t==8) r.params.insert(r.params.end(),v.begin()+i,v.begin()+end);
        i=end;
    }
    return r;
}
bool output(Session& s,std::vector<uint8_t>& bytes) {
    MFT_OUTPUT_STREAM_INFO info{};
    if(FAILED(s.transform->GetOutputStreamInfo(s.out,&info))) return false;
    MFT_OUTPUT_DATA_BUFFER data{}; data.dwStreamID=s.out;
    ComPtr<IMFSample> sample;
    if(!(info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES|MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES))) {
        ComPtr<IMFMediaBuffer> buffer;
        if(info.cbSize>2*1024*1024) return false;
        if(FAILED(MFCreateSample(&sample))
           || FAILED(MFCreateAlignedMemoryBuffer(std::max<DWORD>(info.cbSize,2*1024*1024),
                info.cbAlignment?info.cbAlignment-1:0,&buffer))
           || FAILED(sample->AddBuffer(buffer.Get()))) return false;
        data.pSample=sample.Get();
    }
    DWORD status=0;
    HRESULT hr=s.transform->ProcessOutput(0,1,&data,&status);
    if(data.pEvents) data.pEvents->Release();
    ComPtr<IMFSample> returned;
    if(data.pSample && data.pSample!=sample.Get()) returned.Attach(data.pSample);
    if(FAILED(hr) || !data.pSample) return false;
    ComPtr<IMFMediaBuffer> buffer;
    DWORD count=0;
    if(FAILED(data.pSample->GetBufferCount(&count)) || !count) return false;
    // A single-buffer sample needs no contiguous conversion.
    if(count==1) {
        if(FAILED(data.pSample->GetBufferByIndex(0,&buffer))) return false;
    } else if(FAILED(data.pSample->ConvertToContiguousBuffer(&buffer))) return false;
    BYTE* ptr=nullptr; DWORD len=0;
    if(FAILED(buffer->Lock(&ptr,nullptr,&len))) return false;
    // Unlock even if the vector allocation throws.
    struct Unlock { IMFMediaBuffer* p; ~Unlock(){p->Unlock();} } unlock{buffer.Get()};
    return annex(ptr,len,bytes);
}
bool event(Session& s,MediaEventType& type,ULONGLONG deadline) {
    while(GetTickCount64()<deadline) {
        ComPtr<IMFMediaEvent> e;
        HRESULT hr=s.events->GetEvent(MF_EVENT_FLAG_NO_WAIT,&e);
        if(hr==MF_E_NO_EVENTS_AVAILABLE) { Sleep(1); continue; }
        if(FAILED(hr)) return false;
        HRESULT status=E_FAIL;
        if(FAILED(e->GetStatus(&status)) || FAILED(status) || FAILED(e->GetType(&type))) return false;
        return true;
    }
    return false;
}
bool input(Session& s,const uint8_t* nv12,DWORD n,ID3D11Texture2D* texture) {
    ComPtr<IMFSample> sample; ComPtr<IMFMediaBuffer> buffer;
    if(FAILED(MFCreateSample(&sample))) return false;
    if(texture) {
        if(!s.manager || FAILED(MFCreateDXGISurfaceBuffer(__uuidof(ID3D11Texture2D),texture,0,FALSE,&buffer))) return false;
    } else {
        if(s.manager || !nv12 || FAILED(MFCreateMemoryBuffer(n,&buffer))) return false;
        BYTE* dst=nullptr;
        if(FAILED(buffer->Lock(&dst,nullptr,nullptr))) return false;
        memcpy(dst,nv12,n);buffer->Unlock();
    }
    LONGLONG duration=10000000/s.fps;
    if(FAILED(buffer->SetCurrentLength(n)) || FAILED(sample->AddBuffer(buffer.Get()))
       || FAILED(sample->SetSampleTime(s.pts)) || FAILED(sample->SetSampleDuration(duration))) return false;
    if(s.frame%s.fps==0) set_uint(s.codec.Get(),CODECAPI_AVEncVideoForceKeyFrame,1);
    if(FAILED(s.transform->ProcessInput(s.in,sample.Get(),0))) return false;
    s.pts+=duration; ++s.frame; return true;
}
}
static void* create_encoder(uint32_t w,uint32_t h,uint32_t fps,uint32_t bitrate,ID3D11Device* device) noexcept {
    try {
        if(w<2 || h<2 || w>1280 || h>720 || (w&1) || (h&1) || fps<5 || fps>30) return nullptr;
        auto s=std::make_unique<Session>();
        HRESULT hr=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
        if(FAILED(hr)) return nullptr;
        s->com=true;
        if(FAILED(MFStartup(MF_VERSION,MFSTARTUP_FULL))) return nullptr;
        s->mf=true; s->w=w; s->h=h; s->fps=fps;
        if(device) {
            UINT reset=0;
            if(FAILED(MFCreateDXGIDeviceManager(&reset,&s->manager)) ||
               FAILED(s->manager->ResetDevice(device,reset))) return nullptr;
        }
        MFT_REGISTER_TYPE_INFO in{MFMediaType_Video,MFVideoFormat_NV12};
        MFT_REGISTER_TYPE_INFO out{MFMediaType_Video,MFVideoFormat_H264};
        IMFActivate** list=nullptr; UINT32 count=0;
        hr=MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER,MFT_ENUM_FLAG_HARDWARE|MFT_ENUM_FLAG_SORTANDFILTER,
                     &in,&out,&list,&count);
        bool ok=false;
        if(SUCCEEDED(hr)) {
            // Only a hardware-enumerated asynchronous MFT can be classified hardware.
            for(UINT32 i=0;i<count;++i) {
                if(!ok) {
                    ok=start(*s,list[i],bitrate);
                    if(!ok) { s->codec.Reset(); s->events.Reset(); s->transform.Reset(); list[i]->ShutdownObject(); }
                }
                list[i]->Release();
            }
        }
        CoTaskMemFree(list);
        return ok?s.release():nullptr;
    } catch(...) { return nullptr; }
}
extern "C" void* p2p_mf_create(uint32_t w,uint32_t h,uint32_t fps,uint32_t bitrate) noexcept {
    return create_encoder(w,h,fps,bitrate,nullptr);
}
extern "C" void* p2p_mf_create_gpu(uint32_t w,uint32_t h,uint32_t fps,uint32_t bitrate,void* device) noexcept {
    return device?create_encoder(w,h,fps,bitrate,static_cast<ID3D11Device*>(device)):nullptr;
}
static int encode_frame(void* handle,const uint8_t* nv12,size_t n,ID3D11Texture2D* texture,uint8_t* dst,size_t cap,size_t* written) noexcept {
    try {
        if(!handle || (!nv12 && !texture) || !dst || !written) return -1;
        *written=0;
        auto& s=*static_cast<Session*>(handle);
        if(texture) {
            D3D11_TEXTURE2D_DESC td{};texture->GetDesc(&td);
            if(!s.manager || td.Width!=s.w || td.Height!=s.h || td.Format!=DXGI_FORMAT_NV12) return -1;
            n=size_t(s.w)*s.h*3/2;
        }
        if(n!=size_t(s.w)*s.h*3/2) return -1;
        bool fed=false;
        // One polling budget per access unit, not one per event. Driver
        // calls themselves may block; this is not an end-to-end deadline.
        const auto deadline=GetTickCount64()+180;
        for(unsigned i=0;i<12 && GetTickCount64()<deadline;++i) {
            if(s.need && !fed) {
                if(!input(s,nv12,static_cast<DWORD>(n),texture)) return -1;
                s.need=false; fed=true;
            }
            MediaEventType type;
            if(!event(s,type,deadline)) return -1;
            if(type==METransformNeedInput) { s.need=true; continue; }
            if(type!=METransformHaveOutput) continue;
            if(!fed) return -1; // no stale buffered output may be attributed to this input
            auto& bytes=s.encoded;
            bytes.clear();
            if(!output(s,bytes)) return -1;
            auto nal=scan(bytes);
            if(nal.sps && nal.pps) s.headers=nal.params;
            if(s.frame==1 && !nal.idr) return -1;
            const bool prepend=nal.idr && (!nal.sps || !nal.pps);
            const size_t header_len=prepend?s.headers.size():0;
            if((prepend && !header_len) || bytes.empty() || bytes.size()>cap ||
               header_len>cap-bytes.size()) return -1;
            // Write directly to the caller's output; no vector insertion
            // or shifting the encoded access unit to prepend parameters.
            if(header_len) memcpy(dst,s.headers.data(),header_len);
            memcpy(dst+header_len,bytes.data(),bytes.size());
            *written=header_len+bytes.size(); return 0;
        }
        return -1;
    } catch(...) { return -1; }
}
extern "C" int p2p_mf_encode(void* handle,const uint8_t* nv12,size_t n,uint8_t* dst,size_t cap,size_t* written) noexcept {
    return encode_frame(handle,nv12,n,nullptr,dst,cap,written);
}
extern "C" int p2p_mf_encode_gpu(void* handle,void* texture,uint8_t* dst,size_t cap,size_t* written) noexcept {
    return texture?encode_frame(handle,nullptr,0,static_cast<ID3D11Texture2D*>(texture),dst,cap,written):-1;
}
extern "C" void p2p_mf_destroy(void* handle) noexcept { delete static_cast<Session*>(handle); }

// Exercise the same Annex-B conversion and scratch allocation with bounded
// fixtures. This is not a hardware encoder or performance test.
extern "C" int p2p_mf_test_scratch() noexcept {
    try {
        std::vector<uint8_t> scratch;
        const uint8_t first[]={0,0,0,2,0x67,0x12,0,0,0,2,0x68,0x34,0,0,0,2,0x65,0x56};
        if(!annex(first,sizeof(first),scratch)) return 0;
        auto nal=scan(scratch);
        if(!nal.sps || !nal.pps || !nal.idr) return 0;
        auto* allocation=scratch.data();
        const uint8_t second[]={0,0,0,1,0x41,0x78};
        if(!annex(second,sizeof(second),scratch) || scratch.data()!=allocation ||
           scratch.size()!=sizeof(second) || memcmp(scratch.data(),second,sizeof(second))) return 0;
        nal=scan(scratch);
        if(nal.sps || nal.pps || nal.idr || !nal.params.empty()) return 0;
        const uint8_t invalid[]={0,0,0,9,0x65,0x42};
        if(annex(invalid,sizeof(invalid),scratch)) return 0;
        if(!annex(second,sizeof(second),scratch) || scratch.size()!=sizeof(second)) return 0;
        return 1;
    } catch(...) {return 0;}
}

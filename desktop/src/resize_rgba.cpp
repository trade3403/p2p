// Bounded separable antialiased triangle resampling for CPU AVC capture.
// Fixed-point normalized weights; no platform API, no unbounded pixel cache.
// This translation unit is optimized even in the native debug test profile.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

namespace {
constexpr uint32_t ONE=4096;
struct Tap { uint32_t index, weight; };
using Kernel=std::vector<std::vector<Tap>>;
Kernel make_kernel(uint32_t src,uint32_t dst) {
    Kernel kernel(dst);
    const double scale=double(src)/dst;
    const double support=std::max(1.0,scale);
    for(uint32_t o=0;o<dst;++o) {
        const double center=(o+0.5)*scale-0.5;
        const int left=std::max(0,int(std::floor(center-support)));
        const int right=std::min(int(src)-1,int(std::ceil(center+support)));
        double total=0;
        std::vector<double> weights;
        for(int i=left;i<=right;++i) {
            const double weight=std::max(0.0,1.0-std::abs(i-center)/support);
            if(weight>0) {kernel[o].push_back({uint32_t(i),0});weights.push_back(weight);total+=weight;}
        }
        // Rounding the cumulative sum ensures weights are nonnegative
        // and add to exactly ONE, even at cropped image boundaries.
        double accum=0;
        uint32_t previous=0;
        for(size_t i=0;i<weights.size();++i) {
            accum+=weights[i];
            const uint32_t next=i+1==weights.size()?ONE:
                std::min(ONE,uint32_t(std::lround(accum/total*ONE)));
            kernel[o][i].weight=next-previous;
            previous=next;
        }
    }
    return kernel;
}
}
extern "C" int p2p_resize_rgba_triangle(const uint8_t* input,size_t input_len,
    uint32_t sw,uint32_t sh,uint8_t* output,size_t output_len,uint32_t dw,uint32_t dh) noexcept {
    try {
        // Limit this fast path to bounded downsampling up to 4x per axis;
        // the existing image crate handles all other valid ratios.
        if(!input || !output || dw<2 || dh<2 || sw<dw || sh<dh ||
            sw>8192 || sh>8192 || uint64_t(sw)*sh>34000000 ||
            dw>1280 || dh>720 || sw>4*dw || sh>4*dh ||
            input_len!=size_t(sw)*sh*4 || output_len!=size_t(dw)*dh*4) return 0;
        const auto xkernel=make_kernel(sw,dw);
        const auto ykernel=make_kernel(sh,dh);
        // Vertical pass keeps one full source-width row at Q12 precision.
        // Horizontal pass rounds once at the final Q24 output.
        std::vector<std::array<uint32_t,4>> row(sw);
        for(uint32_t y=0;y<dh;++y) {
            std::fill(row.begin(),row.end(),std::array<uint32_t,4>{});
            for(const auto& tap:ykernel[y]) {
                const uint8_t* src=input+size_t(tap.index)*sw*4;
                const uint32_t weight=tap.weight;
                for(uint32_t x=0;x<sw;++x) {
                    for(unsigned c=0;c<4;++c) row[x][c]+=uint32_t(src[size_t(x)*4+c])*weight;
                }
            }
            uint8_t* dst=output+size_t(y)*dw*4;
            for(uint32_t x=0;x<dw;++x) {
                std::array<uint64_t,4> sum{};
                for(const auto& tap:xkernel[x]) {
                    for(unsigned c=0;c<4;++c) sum[c]+=uint64_t(row[tap.index][c])*tap.weight;
                }
                for(unsigned c=0;c<4;++c)
                    dst[size_t(x)*4+c]=uint8_t(std::min<uint64_t>(255,(sum[c]+(1u<<23))>>24));
            }
        }
        return 1;
    } catch(...) {return 0;}
}

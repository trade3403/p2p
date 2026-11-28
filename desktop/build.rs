fn main() {
    println!("cargo:rerun-if-changed=src/mf_encoder.cpp");
    println!("cargo:rerun-if-changed=src/dxgi_capture.cpp");
    println!("cargo:rerun-if-changed=src/lan_candidates.cpp");
    println!("cargo:rerun-if-changed=src/resize_rgba.cpp");
    #[cfg(windows)]
    {
        cc::Build::new().cpp(true).file("src/mf_encoder.cpp").file("src/dxgi_capture.cpp").file("src/lan_candidates.cpp")
            .flag_if_supported("/std:c++17").compile("p2p_mf_encoder");
        // Pure pixel kernel: same optimized native implementation in debug
        // CI and release, without changing the rest of the test profile.
        cc::Build::new().cpp(true).file("src/resize_rgba.cpp").opt_level(3)
            .flag_if_supported("/std:c++17").compile("p2p_resize_rgba");
        for lib in ["mfplat","mf","mfuuid","ole32","oleaut32","wmcodecdspuuid","d3d11","dxgi","user32","iphlpapi","ws2_32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}

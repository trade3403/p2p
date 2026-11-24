fn main() {
    println!("cargo:rerun-if-changed=src/mf_encoder.cpp");
    println!("cargo:rerun-if-changed=src/dxgi_capture.cpp");
    #[cfg(windows)]
    {
        cc::Build::new().cpp(true).file("src/mf_encoder.cpp").file("src/dxgi_capture.cpp")
            .flag_if_supported("/std:c++17").compile("p2p_mf_encoder");
        for lib in ["mfplat","mf","mfuuid","ole32","oleaut32","wmcodecdspuuid","d3d11","dxgi","user32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
}

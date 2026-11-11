//! Builds Haiku's C++ shim (src/platform/haiku_shim.cpp). Nothing to do on other systems.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/platform/haiku_shim.cpp");
    // build scripts run on the build machine; Haiku is only ever built natively
    #[cfg(target_os = "haiku")]
    {
        cc::Build::new()
            .cpp(true)
            .file("src/platform/haiku_shim.cpp")
            .compile("haikufetch_shim");
        println!("cargo:rustc-link-lib=be");
        println!("cargo:rustc-link-lib=bnetapi");
        println!("cargo:rustc-link-lib=stdc++");
    }
}

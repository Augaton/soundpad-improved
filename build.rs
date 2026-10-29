// Embeds the microphone filter (native\bin\SoundpadImprovedApo.dll, built by native\build.cmd)
// when it exists. Without it the app still builds; installing the filter then reports it.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(apo_dll)");
    println!("cargo::rerun-if-changed=native/bin");
    let dll = std::path::Path::new("native/bin/SoundpadImprovedApo.dll");
    if let Ok(path) = dll.canonicalize() {
        println!("cargo::rerun-if-changed=native/bin/SoundpadImprovedApo.dll");
        println!("cargo::rustc-cfg=apo_dll");
        println!("cargo::rustc-env=SPI_APO_DLL={}", path.display());
    }
}

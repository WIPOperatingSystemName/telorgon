#[cfg(feature = "shell-wayland-linux")]
#[path = "build/wayland.rs"]
mod wayland;

#[cfg(feature = "shell-xwayland-embedded")]
#[path = "src/integrations/x11/payload_format.rs"]
mod payload_format;

#[cfg(feature = "shell-xwayland-embedded")]
#[path = "build/xwayland.rs"]
mod xwayland;

fn main() {
    #[cfg(feature = "shell-xwayland-embedded")]
    embed_xwayland();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src/integrations/wayland/server/protocol.rs");
    #[cfg(feature = "shell-wayland-linux")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Err(error) = wayland::generate() {
            panic!("Wayland descriptor generation failed: {error}");
        }
    }
}

#[cfg(feature = "shell-xwayland-embedded")]
fn embed_xwayland() {
    println!("cargo:rerun-if-env-changed=TELORGON_XWAYLAND_PAYLOAD");
    println!("cargo:rerun-if-changed=src/integrations/x11/payload_format.rs");
    let path = xwayland::payload_path();
    assert!(
        path.is_absolute(),
        "TELORGON_XWAYLAND_PAYLOAD must be absolute"
    );
    println!("cargo:rerun-if-changed={}", path.display());
    let size = std::fs::metadata(&path)
        .expect("cannot stat Xwayland payload")
        .len();
    assert!(
        size <= payload_format::MAX_COMPRESSED as u64,
        "Xwayland payload exceeds archive limit"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .expect("cannot open Xwayland payload")
        .take(payload_format::MAX_COMPRESSED as u64 + 1)
        .read_to_end(&mut bytes)
        .expect("cannot read Xwayland payload");
    payload_format::validate(&bytes, &std::env::var("TARGET").unwrap())
        .unwrap_or_else(|e| panic!("invalid Xwayland payload: {e}"));
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("xwayland.payload"), bytes)
        .expect("cannot stage validated Xwayland payload");
}

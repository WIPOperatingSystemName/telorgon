#[cfg(feature = "desktop-wayland-linux")]
#[path = "build/wayland.rs"]
mod wayland;

#[cfg(feature = "desktop-xwayland-embedded")]
#[path = "src/xwayland/payload_format.rs"]
mod payload_format;

fn main() {
    #[cfg(feature = "desktop-xwayland-embedded")]
    embed_xwayland();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src/wayland_server/protocol.rs");
    #[cfg(feature = "desktop-wayland-linux")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Err(error) = wayland::generate() {
            panic!("Wayland descriptor generation failed: {error}");
        }
    }
}

#[cfg(feature = "desktop-xwayland-embedded")]
fn embed_xwayland() {
    println!("cargo:rerun-if-env-changed=TELORGON_XWAYLAND_PAYLOAD");
    println!("cargo:rerun-if-changed=src/xwayland/payload_format.rs");
    let path = std::env::var_os("TELORGON_XWAYLAND_PAYLOAD")
        .expect("desktop-xwayland-embedded requires TELORGON_XWAYLAND_PAYLOAD; see packaging/xwayland/README.md");
    let path = std::path::PathBuf::from(path);
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

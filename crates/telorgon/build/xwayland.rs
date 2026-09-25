//! Resolve an explicit payload or prepare the cached native dependency on demand.
use std::{env, path::PathBuf, process::Command};

pub fn payload_path() -> PathBuf {
    for variable in [
        "TELORGON_XWAYLAND_PAYLOAD",
        "TELORGON_XWAYLAND_BUILD_ROOT",
        "TELORGON_XWAYLAND_CACHE",
        "CARGO_NET_OFFLINE",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    if let Some(path) = env::var_os("TELORGON_XWAYLAND_PAYLOAD").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository = manifest.parent().unwrap().parent().unwrap();
    for relative in [
        "tools/sdk/ensure_xwayland.py",
        "tools/sdk/build_xwayland.py",
        "tools/sdk/source_cache.py",
        "third_party/sources.lock.toml",
        "third_party/recipes/xwayland",
        "third_party/patches/xwayland",
        "packaging/linux/xwayland/stage.py",
        "packaging/linux/xwayland/pack.py",
        "packaging/linux/xwayland/runtime-policy.toml",
    ] {
        println!("cargo:rerun-if-changed={}", repository.join(relative).display());
    }
    let root = env::var_os("TELORGON_XWAYLAND_BUILD_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join("target/xwayland-build/root"));
    for relative in ["var/lib/dpkg/status", "etc/os-release", "usr/local/bin/cmake"] {
        let path = root.join(relative);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let result = Command::new("python3")
        .arg(repository.join("tools/sdk/ensure_xwayland.py"))
        .arg("--target")
        .arg(env::var("TARGET").unwrap())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("automatic Xwayland preparation requires python3 in PATH");
    if !result.status.success() {
        panic!("automatic Xwayland preparation failed:\n{}", String::from_utf8_lossy(&result.stderr));
    }
    for line in String::from_utf8_lossy(&result.stderr).lines() {
        println!("cargo:warning={line}");
    }
    let path = PathBuf::from(String::from_utf8(result.stdout)
        .expect("Xwayland preparation returned a non-UTF-8 path").trim());
    assert!(path.is_absolute(), "Xwayland preparation returned a non-absolute path");
    println!("cargo:rerun-if-changed={}", path.with_file_name("build.json").display());
    path
}

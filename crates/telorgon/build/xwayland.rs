//! Resolve an explicit payload or prepare the cached native dependency on demand.
use std::{env, path::PathBuf, process::Command};

pub struct SourcePin {
    pub target: String,
    pub version: String,
    pub sha256: String,
}

pub fn source_pin() -> SourcePin {
    #[derive(serde::Deserialize)]
    struct Sources {
        schema: u32,
        target: String,
        source: Vec<Source>,
    }
    #[derive(serde::Deserialize)]
    struct Source {
        name: String,
        version: String,
        sha256: String,
    }
    let path = repository().join("third_party/sources.lock.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let lock: Sources =
        toml::from_str(&std::fs::read_to_string(&path).expect("cannot read native source lock"))
            .expect("invalid native source lock");
    assert!(
        lock.schema == 1 && lock.target == env::var("TARGET").unwrap(),
        "native source lock does not support the Cargo target"
    );
    let mut sources = lock
        .source
        .into_iter()
        .filter(|source| source.name == "xwayland");
    let source = sources.next().expect("native source lock lacks Xwayland");
    assert!(sources.next().is_none(), "duplicate Xwayland source pin");
    assert!(
        super::payload_format::xwayland_version(&source.version)
            && source.sha256.len() == 64
            && source
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid Xwayland source version or checksum"
    );
    SourcePin {
        target: lock.target,
        version: source.version,
        sha256: source.sha256,
    }
}

fn repository() -> PathBuf {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    manifest.parent().unwrap().parent().unwrap().to_path_buf()
}

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
    let repository = repository();
    for relative in [
        "tools/sdk/ensure_xwayland.py",
        "tools/sdk/build_xwayland.py",
        "tools/sdk/source_cache.py",
        "tools/sdk/native_inputs.py",
        "third_party/sources.lock.toml",
        "third_party/recipes/xwayland",
        "third_party/patches/xwayland",
        "packaging/linux/xwayland/stage.py",
        "packaging/linux/xwayland/pack.py",
        "packaging/linux/xwayland/abi_audit.py",
        "packaging/linux/xwayland/runtime-policy.toml",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            repository.join(relative).display()
        );
    }
    if let Some(root) = env::var_os("TELORGON_XWAYLAND_BUILD_ROOT").filter(|path| !path.is_empty())
    {
        for relative in [
            "var/lib/dpkg/status",
            "etc/os-release",
            "usr/local/bin/cmake",
        ] {
            let path = PathBuf::from(&root).join(relative);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
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
        panic!(
            "automatic Xwayland preparation failed:\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for line in String::from_utf8_lossy(&result.stderr).lines() {
        println!("cargo:warning={line}");
    }
    let path = PathBuf::from(
        String::from_utf8(result.stdout)
            .expect("Xwayland preparation returned a non-UTF-8 path")
            .trim(),
    );
    assert!(
        path.is_absolute(),
        "Xwayland preparation returned a non-absolute path"
    );
    let marker = path.with_file_name("build.json");
    println!("cargo:rerun-if-changed={}", marker.display());
    if let Ok(bytes) = std::fs::read(&marker) {
        let marker: serde_json::Value =
            serde_json::from_slice(&bytes).expect("invalid Xwayland build cache marker");
        if let Some(inputs) = marker.get("inputs").and_then(serde_json::Value::as_array) {
            for input in inputs {
                let input = input.as_str().expect("invalid Xwayland cache input path");
                assert!(
                    PathBuf::from(input).is_absolute() && !input.contains(['\n', '\r']),
                    "invalid Xwayland cache input path"
                );
                println!("cargo:rerun-if-changed={input}");
            }
        }
    }
    path
}

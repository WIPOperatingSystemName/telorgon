//! Shared by the offline build validator and runtime extractor.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;

pub const MAGIC: &[u8; 8] = b"TLXWP001";
pub const MAX_COMPRESSED: usize = 64 * 1024 * 1024;
pub const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
pub const MAX_MANIFEST: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 16_384;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub target: String,
    pub minimum_glibc: String,
    pub xwayland_version: String,
    pub components: Vec<Component>,
    pub entries: Vec<Entry>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub name: String,
    pub version: String,
    pub input_sha256: String,
    pub license: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub mode: u32,
    pub offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub sha256: String,
}
pub fn hash(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(result, "{byte:02x}").unwrap();
    }
    result
}
fn digest_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        })
}
pub fn parse<'a>(bytes: &'a [u8], target: &str) -> Result<(Manifest, &'a [u8]), String> {
    if bytes.len() > MAX_COMPRESSED || bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err("invalid or oversized Xwayland payload".into());
    }
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if n > MAX_MANIFEST || n > bytes.len() - 12 {
        return Err("invalid manifest length".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes[12..12 + n]).map_err(|_| "invalid payload manifest JSON")?;
    if manifest.schema != 1
        || manifest.target != target
        || target != "x86_64-unknown-linux-gnu"
        || manifest.minimum_glibc != "2.39"
        || manifest.xwayland_version != "24.1.13"
        || manifest.entries.is_empty()
        || manifest.entries.len() > MAX_ENTRIES
        || manifest.components.is_empty()
        || manifest.components.len() > 256
    {
        return Err("unsupported payload schema, ABI, version, or count".into());
    }
    let mut components = BTreeSet::new();
    for c in &manifest.components {
        if !valid_path(&c.name)
            || c.version.is_empty()
            || c.version.len() > 128
            || c.license.is_empty()
            || c.license.len() > 1024
            || !digest_valid(&c.input_sha256)
            || !components.insert(&c.name)
        {
            return Err("invalid component provenance".into());
        }
    }
    let data = &bytes[12 + n..];
    let mut paths = BTreeSet::new();
    let (mut end, mut expanded) = (0_u64, 0_u64);
    for e in &manifest.entries {
        if !valid_path(&e.path)
            || !matches!(e.mode, 0o400 | 0o500)
            || e.offset != end
            || e.compressed_size == 0
            || !digest_valid(&e.sha256)
            || !paths.insert(e.path.as_str())
        {
            return Err("invalid, overlapping or duplicate payload entry".into());
        }
        end = end
            .checked_add(e.compressed_size)
            .ok_or("payload offset overflow")?;
        expanded = expanded
            .checked_add(e.size)
            .ok_or("payload size overflow")?;
        if end > data.len() as u64 || expanded > MAX_EXPANDED {
            return Err("payload bounds exceeded".into());
        }
    }
    if end != data.len() as u64 {
        return Err("unaccounted payload trailing bytes".into());
    }
    for path in &paths {
        for (i, _) in path.match_indices('/') {
            if paths.contains(&path[..i]) {
                return Err("file is also a directory prefix".into());
            }
        }
    }
    for path in ["bin/Xwayland", "bin/xkbcomp"] {
        if !manifest
            .entries
            .iter()
            .any(|e| e.path == path && e.mode == 0o500)
        {
            return Err("payload lacks a required executable".into());
        }
    }
    if !paths.iter().any(|p| p.starts_with("share/X11/xkb/"))
        || !paths.iter().any(|p| p.starts_with("licenses/"))
    {
        return Err("payload lacks keyboard data or notices".into());
    }
    Ok((manifest, data))
}
pub fn decode(e: &Entry, data: &[u8]) -> Result<Vec<u8>, String> {
    let start = usize::try_from(e.offset).map_err(|_| "invalid offset")?;
    let len = usize::try_from(e.compressed_size).map_err(|_| "invalid length")?;
    let end = start.checked_add(len).ok_or("invalid range")?;
    let compressed = data.get(start..end).ok_or("invalid entry range")?;
    let mut decoder = flate2::read::DeflateDecoder::new(compressed);
    let mut out = Vec::new();
    (&mut decoder)
        .take(e.size + 1)
        .read_to_end(&mut out)
        .map_err(|_| "corrupt deflate entry")?;
    if out.len() as u64 != e.size
        || decoder.total_in() != e.compressed_size
        || hash(&out) != e.sha256
    {
        return Err("payload entry size or digest mismatch".into());
    }
    if e.path.starts_with("bin/")
        && (out.len() < 64
            || &out[..6] != b"\x7fELF\x02\x01"
            || u16::from_le_bytes([out[18], out[19]]) != 62)
    {
        return Err("payload executable is not x86-64 ELF".into());
    }
    Ok(out)
}
pub fn validate(bytes: &[u8], target: &str) -> Result<Manifest, String> {
    let (manifest, data) = parse(bytes, target)?;
    for e in &manifest.entries {
        decode(e, data)?;
    }
    Ok(manifest)
}

#[cfg(test)]
pub(crate) fn fixture() -> Vec<u8> {
    use std::io::Write;
    // Synthetic ELF headers are test data only; these files are never executed.
    let mut elf = vec![0u8; 64];
    elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
    elf[18] = 62;
    let mut entries = Vec::new();
    let mut data = Vec::new();
    for (path, mode, raw) in [
        ("bin/Xwayland", 0o500, elf.as_slice()),
        ("bin/xkbcomp", 0o500, elf.as_slice()),
        (
            "licenses/test.txt",
            0o400,
            b"synthetic test data only".as_slice(),
        ),
        (
            "share/X11/xkb/symbols/test",
            0o400,
            b"synthetic keyboard data".as_slice(),
        ),
    ] {
        let mut compressor =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        compressor.write_all(raw).unwrap();
        let compressed = compressor.finish().unwrap();
        entries.push(
            serde_json::json!({"path":path,"mode":mode,"offset":data.len(),
            "compressed_size":compressed.len(),"size":raw.len(),"sha256":hash(raw)}),
        );
        data.extend(compressed);
    }
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema":1,"target":"x86_64-unknown-linux-gnu","minimum_glibc":"2.39",
        "xwayland_version":"24.1.13", "components":[{"name":"test-only",
        "version":"0","input_sha256":hash(b"test"),"license":"synthetic fixture"}],"entries":entries
    }))
    .unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend((manifest.len() as u32).to_le_bytes());
    bytes.extend(manifest);
    bytes.extend(data);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_corruption_truncation_and_trailing_bytes() {
        let bytes = fixture();
        assert!(validate(&bytes, "x86_64-unknown-linux-gnu").is_ok());
        for length in [0, 8, 12, bytes.len() - 1] {
            assert!(validate(&bytes[..length], "x86_64-unknown-linux-gnu").is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(validate(&trailing, "x86_64-unknown-linux-gnu").is_err());
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        assert!(validate(&corrupt, "x86_64-unknown-linux-gnu").is_err());
        assert!(validate(&bytes, "aarch64-unknown-linux-gnu").is_err());
    }
    #[test]
    fn rejects_escaping_and_ambiguous_paths() {
        for path in [
            "", "/bin/x", "../x", "bin/../x", "bin//x", "bin/./x", "bin/x\0", "bin/a\\b",
        ] {
            assert!(!valid_path(path), "{path}");
        }
    }
    #[test]
    fn manifest_bounds_checked_before_decompression() {
        let bytes = fixture();
        let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let original: serde_json::Value = serde_json::from_slice(&bytes[12..12 + n]).unwrap();
        for (field, value) in [
            ("size", serde_json::json!(MAX_EXPANDED + 1)),
            ("offset", serde_json::json!(u64::MAX)),
            ("path", serde_json::json!("../escape")),
            ("mode", serde_json::json!(0o777)),
        ] {
            let mut manifest = original.clone();
            manifest["entries"][0][field] = value;
            let encoded = serde_json::to_vec(&manifest).unwrap();
            let mut changed = MAGIC.to_vec();
            changed.extend((encoded.len() as u32).to_le_bytes());
            changed.extend(encoded);
            changed.extend(&bytes[12 + n..]);
            assert!(
                validate(&changed, "x86_64-unknown-linux-gnu").is_err(),
                "{field}"
            );
        }
    }
}

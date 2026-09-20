mod compile;
mod generate_rust;
mod manifest;
mod reflect;
mod validate;

use std::fs;
use std::path::{Path, PathBuf};

use manifest::{BundleSource, GeneratedBundle, GeneratedShader};
use sha2::{Digest, Sha256};

fn main() {
    if let Err(error) = run() {
        eprintln!("telorgon-shader-build: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let renderer_dir = crate_dir
        .parent()
        .ok_or("shader-build package has no crates directory")?
        .join("telorgon/src/graphics/renderers/vulkan");
    let shader_dir = renderer_dir.join("shaders");
    let bundle_path = shader_dir.join("bundle.toml");
    let generated_dir = shader_dir.join("generated");
    let artifact_dir = generated_dir.join("spirv");
    fs::create_dir_all(&artifact_dir).map_err(|error| error.to_string())?;

    let bundle_text = fs::read_to_string(&bundle_path).map_err(|error| error.to_string())?;
    let source: BundleSource = toml::from_str(&bundle_text).map_err(|error| error.to_string())?;
    if source.schema_version != 1 || source.interface_major != 4 {
        return Err("unsupported shader bundle or interface version".to_owned());
    }
    if source.target != "vulkan1.3-spirv1.6" {
        return Err(format!("unsupported target {:?}", source.target));
    }

    let mut generated = Vec::with_capacity(source.shader.len());
    let mut bundle_hasher = Sha256::new();
    for shader in &source.shader {
        let source_path = shader_dir.join(&shader.source);
        let text = fs::read_to_string(&source_path).map_err(|error| error.to_string())?;
        let words = compile::compile(&text, &shader.source, &shader.stage)?;
        validate::validate(&words)?;
        let bytes = words_as_bytes(&words);
        reflect::verify_interface(bytes, &shader.stage, &shader.name)?;
        let source_hash = hash(text.as_bytes());
        let artifact_hash = hash(bytes);
        bundle_hasher.update(shader.name.as_bytes());
        bundle_hasher.update(artifact_hash.as_bytes());
        fs::write(artifact_dir.join(&shader.artifact), bytes).map_err(|error| error.to_string())?;
        generated.push(GeneratedShader {
            name: shader.name.clone(),
            source: shader.source.clone(),
            source_hash,
            artifact: shader.artifact.clone(),
            artifact_hash,
            stage: shader.stage.clone(),
            entry_point: "main".to_owned(),
        });
    }

    let source_config_hash = hash(bundle_text.as_bytes());
    bundle_hasher.update(source_config_hash.as_bytes());
    let bundle_hash = hex(bundle_hasher.finalize().as_slice());
    let bundle = GeneratedBundle {
        schema_version: source.schema_version,
        interface_major: source.interface_major,
        interface_minor: source.interface_minor,
        target: source.target,
        source_config_hash,
        bundle_hash,
        shader: generated,
    };
    let manifest_text = toml::to_string_pretty(&bundle).map_err(|error| error.to_string())?;
    fs::write(generated_dir.join("manifest.toml"), manifest_text)
        .map_err(|error| error.to_string())?;
    let artifact_dir_from_source = Path::new("spirv");
    fs::write(
        generated_dir.join("metadata.rs"),
        generate_rust::source(&bundle, artifact_dir_from_source),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn words_as_bytes(words: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), std::mem::size_of_val(words)) }
}

fn hash(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_slice())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

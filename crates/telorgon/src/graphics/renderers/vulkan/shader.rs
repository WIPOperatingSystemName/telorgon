use crate::graphics::gpu_abi::{GPU_ABI_MAJOR, GPU_ABI_MINOR};
use crate::graphics::render::{PipelineKind, RenderResult};
use ash::vk;
use sha2::{Digest, Sha256};

use crate::graphics::renderers::vulkan::error::{internal, vk_error};
use crate::graphics::renderers::vulkan::generated_shader_bundle::{
    BLUR_FRAGMENT, BLUR_FRAGMENT_HASH, BLUR_VERTEX, BLUR_VERTEX_HASH,
    BOX_FRAGMENT, BOX_FRAGMENT_HASH, BOX_VERTEX, BOX_VERTEX_HASH, BUNDLE_HASH,
    BUNDLE_INTERFACE_MAJOR, BUNDLE_INTERFACE_MINOR, GLYPH_FRAGMENT, GLYPH_FRAGMENT_HASH,
    GLYPH_VERTEX, GLYPH_VERTEX_HASH, IMAGE_FRAGMENT, IMAGE_FRAGMENT_HASH, IMAGE_VERTEX,
    IMAGE_VERTEX_HASH, LIQUID_FRAGMENT, LIQUID_FRAGMENT_HASH, LIQUID_VERTEX,
    LIQUID_VERTEX_HASH, MATERIAL_FRAGMENT, MATERIAL_FRAGMENT_HASH, MATERIAL_VERTEX,
    MATERIAL_VERTEX_HASH,
};

pub(crate) struct ShaderModules {
    device: ash::Device,
    pub(crate) vertex: vk::ShaderModule,
    pub(crate) fragment: vk::ShaderModule,
}

impl ShaderModules {
    pub(crate) fn load(device: &ash::Device, pipeline: PipelineKind) -> RenderResult<Self> {
        verify_bundle_metadata()?;
        let (name, vertex_bytes, vertex_hash, fragment_bytes, fragment_hash) = match pipeline {
            PipelineKind::GaussianBlur => ("blur", BLUR_VERTEX, BLUR_VERTEX_HASH, BLUR_FRAGMENT, BLUR_FRAGMENT_HASH),
            PipelineKind::LiquidGlass => (
                "liquid",
                LIQUID_VERTEX,
                LIQUID_VERTEX_HASH,
                LIQUID_FRAGMENT,
                LIQUID_FRAGMENT_HASH,
            ),
            PipelineKind::AnalyticBox => (
                "box",
                BOX_VERTEX,
                BOX_VERTEX_HASH,
                BOX_FRAGMENT,
                BOX_FRAGMENT_HASH,
            ),
            PipelineKind::Glyph => (
                "glyph",
                GLYPH_VERTEX,
                GLYPH_VERTEX_HASH,
                GLYPH_FRAGMENT,
                GLYPH_FRAGMENT_HASH,
            ),
            PipelineKind::Image => (
                "image",
                IMAGE_VERTEX,
                IMAGE_VERTEX_HASH,
                IMAGE_FRAGMENT,
                IMAGE_FRAGMENT_HASH,
            ),
            PipelineKind::Material => (
                "material",
                MATERIAL_VERTEX,
                MATERIAL_VERTEX_HASH,
                MATERIAL_FRAGMENT,
                MATERIAL_FRAGMENT_HASH,
            ),
        };
        verify_hash(&format!("{name} vertex"), vertex_bytes, vertex_hash)?;
        verify_hash(&format!("{name} fragment"), fragment_bytes, fragment_hash)?;
        let vertex = create_module(device, vertex_bytes, &format!("{name} vertex"))?;
        let fragment = match create_module(device, fragment_bytes, &format!("{name} fragment")) {
            Ok(module) => module,
            Err(error) => {
                unsafe { device.destroy_shader_module(vertex, None) };
                return Err(error);
            }
        };
        Ok(Self {
            device: device.clone(),
            vertex,
            fragment,
        })
    }
}

impl Drop for ShaderModules {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.fragment, None);
            self.device.destroy_shader_module(self.vertex, None);
        }
    }
}

fn verify_bundle_metadata() -> RenderResult<()> {
    if BUNDLE_INTERFACE_MAJOR != GPU_ABI_MAJOR || BUNDLE_INTERFACE_MINOR != GPU_ABI_MINOR {
        return Err(internal(format!(
            "shader bundle ABI {}.{} does not match host ABI {}.{}",
            BUNDLE_INTERFACE_MAJOR, BUNDLE_INTERFACE_MINOR, GPU_ABI_MAJOR, GPU_ABI_MINOR
        )));
    }
    if BUNDLE_HASH.len() != 64 {
        return Err(internal("shader bundle hash metadata is malformed"));
    }
    Ok(())
}

fn create_module(device: &ash::Device, bytes: &[u8], name: &str) -> RenderResult<vk::ShaderModule> {
    if !bytes.len().is_multiple_of(4) {
        return Err(internal(format!("{name} SPIR-V byte length is invalid")));
    }
    let words = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("four-byte SPIR-V word")))
        .collect::<Vec<_>>();
    unsafe {
        device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
    }
    .map_err(|result| vk_error(format!("failed to create {name} shader module"), result))
}

fn verify_hash(name: &str, bytes: &[u8], expected: &str) -> RenderResult<()> {
    let actual = hex(Sha256::digest(bytes).as_slice());
    if actual != expected {
        return Err(internal(format!(
            "{name} shader hash mismatch: expected {expected}, found {actual}"
        )));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_shader_hashes_match_generated_metadata() {
        verify_bundle_metadata().unwrap();
        for (name, bytes, hash) in [
            ("blur vertex", BLUR_VERTEX, BLUR_VERTEX_HASH),
            ("blur fragment", BLUR_FRAGMENT, BLUR_FRAGMENT_HASH),
            ("liquid vertex", LIQUID_VERTEX, LIQUID_VERTEX_HASH),
            ("liquid fragment", LIQUID_FRAGMENT, LIQUID_FRAGMENT_HASH),
            ("box vertex", BOX_VERTEX, BOX_VERTEX_HASH),
            ("box fragment", BOX_FRAGMENT, BOX_FRAGMENT_HASH),
            ("glyph vertex", GLYPH_VERTEX, GLYPH_VERTEX_HASH),
            ("glyph fragment", GLYPH_FRAGMENT, GLYPH_FRAGMENT_HASH),
            ("image vertex", IMAGE_VERTEX, IMAGE_VERTEX_HASH),
            ("image fragment", IMAGE_FRAGMENT, IMAGE_FRAGMENT_HASH),
            ("material vertex", MATERIAL_VERTEX, MATERIAL_VERTEX_HASH),
            (
                "material fragment",
                MATERIAL_FRAGMENT,
                MATERIAL_FRAGMENT_HASH,
            ),
        ] {
            verify_hash(name, bytes, hash).unwrap();
        }
    }
}

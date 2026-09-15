use rspirv::dr;
use rspirv::dr::Operand;
use rspirv::spirv::{Decoration, Op, StorageClass};

pub fn verify_interface(
    bytes: &[u8],
    expected_stage: &str,
    shader_name: &str,
) -> Result<(), String> {
    let module = dr::load_bytes(bytes).map_err(|error| error.to_string())?;
    let entry_points = module
        .entry_points
        .iter()
        .filter(|instruction| instruction.class.opcode == Op::EntryPoint)
        .count();
    if entry_points != 1 {
        return Err(format!("expected one entry point, found {entry_points}"));
    }
    if module.types_global_values.iter().any(|instruction| {
        instruction.class.opcode == Op::Variable
            && instruction.operands.iter().any(|operand| {
                matches!(
                    operand,
                    rspirv::dr::Operand::StorageClass(StorageClass::PushConstant)
                )
            })
    }) {
        return Err("push constants are forbidden by the GPU ABI".to_owned());
    }
    if expected_stage != "vertex" && expected_stage != "fragment" {
        return Err(format!("unexpected stage {expected_stage:?}"));
    }
    // Every stage must agree on the 192-byte view block, including output-space rounded clips.
    let mut offsets =
        std::collections::BTreeMap::<u32, std::collections::BTreeMap<u32, u32>>::new();
    for instruction in &module.annotations {
        if instruction.class.opcode == Op::MemberDecorate {
            if let [
                Operand::IdRef(id),
                Operand::LiteralBit32(member),
                Operand::Decoration(Decoration::Offset),
                Operand::LiteralBit32(offset),
            ] = instruction.operands.as_slice()
            {
                offsets.entry(*id).or_default().insert(*member, *offset);
            }
        }
    }
    let expected_offsets = [0, 16, 32, 48, 64, 80, 96, 112, 128, 160];
    if !offsets
        .values()
        .any(|members| members.values().copied().eq(expected_offsets))
    {
        return Err(format!(
            "{shader_name} is missing the GPU ABI 3 view-block layout"
        ));
    }
    let expected_instance_stride = match shader_name.split_once('_').map(|value| value.0) {
        Some("box") => 160,
        Some("glyph" | "image" | "material" | "liquid" | "blur") => 64,
        _ => return Err(format!("unexpected shader name {shader_name:?}")),
    };
    let array_strides = module.annotations.iter().filter_map(|instruction| {
        if instruction.class.opcode != Op::Decorate {
            return None;
        }
        match instruction.operands.as_slice() {
            [
                Operand::IdRef(_),
                Operand::Decoration(Decoration::ArrayStride),
                Operand::LiteralBit32(stride),
            ] => Some(*stride),
            _ => None,
        }
    });
    // Liquid fragments receive clip/opacity as flat inputs and no longer read instances.
    if shader_name != "liquid_fragment"
        && !array_strides
            .into_iter()
            .any(|stride| stride == expected_instance_stride)
    {
        return Err(format!(
            "{shader_name} does not declare its required {expected_instance_stride}-byte instance array stride"
        ));
    }
    if shader_name.starts_with("liquid_") || shader_name.starts_with("blur_") {
        // Base glass optics use flat varyings; the blur fragment reads its kernel.
        // Both pipelines keep the four-set contract with owned sampled images.
        let mut descriptors = std::collections::BTreeMap::<u32, (Option<u32>, Option<u32>)>::new();
        for instruction in &module.annotations {
            if instruction.class.opcode == Op::Decorate {
                if let [
                    Operand::IdRef(id),
                    Operand::Decoration(decoration),
                    Operand::LiteralBit32(value),
                ] = instruction.operands.as_slice()
                {
                    match decoration {
                        Decoration::DescriptorSet => {
                            descriptors.entry(*id).or_default().0 = Some(*value)
                        }
                        Decoration::Binding => descriptors.entry(*id).or_default().1 = Some(*value),
                        _ => {}
                    }
                }
            }
        }
        let actual = descriptors
            .values()
            .filter_map(|(set, binding)| Some(((*set)?, (*binding)?)))
            .collect::<std::collections::BTreeSet<_>>();
        let expected: &[(u32, u32)] = if shader_name == "blur_vertex" {
            &[(0, 0), (1, 0), (1, 2), (2, 0)]
        } else if shader_name == "blur_fragment" {
            &[(0, 0), (1, 1), (2, 0), (2, 1), (3, 0)]
        } else if expected_stage == "vertex" {
            &[(0, 0), (1, 0), (1, 2), (2, 0), (2, 1)]
        } else {
            &[(0, 0), (1, 1), (3, 0), (3, 1)]
        };
        if actual != expected.iter().copied().collect() {
            return Err(format!(
                "{shader_name} has unexpected descriptor bindings: {actual:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rspirv::binary::Assemble;

    const VERTEX: &[u8] =
        include_bytes!("../../telorgon/src/renderer_vulkan/shaders/vulkan/liquid.vert.spv");
    const FRAGMENT: &[u8] =
        include_bytes!("../../telorgon/src/renderer_vulkan/shaders/vulkan/liquid.frag.spv");

    #[test]
    fn liquid_bundle_matches_descriptor_contract_and_uses_explicit_lod() {
        verify_interface(VERTEX, "vertex", "liquid_vertex").unwrap();
        verify_interface(FRAGMENT, "fragment", "liquid_fragment").unwrap();
        let module = dr::load_bytes(FRAGMENT).unwrap();
        let operations = module
            .functions
            .iter()
            .flat_map(|f| &f.blocks)
            .flat_map(|b| &b.instructions)
            .map(|i| i.class.opcode)
            .collect::<Vec<_>>();
        assert!(!operations.contains(&Op::ImageSampleImplicitLod));
        // One blurred sample, one shared sharp center, and two dispersed channels.
        assert_eq!(
            operations
                .iter()
                .filter(|op| **op == Op::ImageSampleExplicitLod)
                .count(),
            4
        );
    }

    #[test]
    fn gaussian_bundle_uses_the_kernel_buffer_and_explicit_lod() {
        let vertex =
            include_bytes!("../../telorgon/src/renderer_vulkan/shaders/vulkan/blur.vert.spv");
        let fragment =
            include_bytes!("../../telorgon/src/renderer_vulkan/shaders/vulkan/blur.frag.spv");
        verify_interface(vertex, "vertex", "blur_vertex").unwrap();
        verify_interface(fragment, "fragment", "blur_fragment").unwrap();
        let module = dr::load_bytes(fragment).unwrap();
        let ops: Vec<_> = module
            .functions
            .iter()
            .flat_map(|f| &f.blocks)
            .flat_map(|b| &b.instructions)
            .map(|i| i.class.opcode)
            .collect();
        assert!(!ops.contains(&Op::ImageSampleImplicitLod));
        assert_eq!(
            ops.iter()
                .filter(|op| **op == Op::ImageSampleExplicitLod)
                .count(),
            3
        );
    }

    #[test]
    fn liquid_reflection_rejects_a_misbound_optics_buffer() {
        let mut module = dr::load_bytes(VERTEX).unwrap();
        let parameter = module
            .annotations
            .iter()
            .find_map(|i| match i.operands.as_slice() {
                [
                    Operand::IdRef(id),
                    Operand::Decoration(Decoration::Binding),
                    Operand::LiteralBit32(1),
                ] => Some(*id),
                _ => None,
            })
            .unwrap();
        for instruction in &mut module.annotations {
            if instruction.operands.first() == Some(&Operand::IdRef(parameter))
                && instruction.operands.get(1) == Some(&Operand::Decoration(Decoration::Binding))
            {
                instruction.operands[2] = Operand::LiteralBit32(7);
            }
        }
        assert!(
            verify_interface(
                crate::words_as_bytes(&module.assemble()),
                "vertex",
                "liquid_vertex"
            )
            .unwrap_err()
            .contains("descriptor bindings")
        );
    }
}

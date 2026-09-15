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
        Some("glyph" | "image" | "material" | "liquid") => 64,
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
    if !array_strides
        .into_iter()
        .any(|stride| stride == expected_instance_stride)
    {
        return Err(format!(
            "{shader_name} does not declare its required {expected_instance_stride}-byte instance array stride"
        ));
    }
    if shader_name.starts_with("liquid_") {
        // The optics block is vertex-only; flat varyings carry it to fragments.
        // Keep the existing four-set contract and single owned sampled backdrop.
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
        let expected: &[(u32, u32)] = if expected_stage == "vertex" {
            &[(0, 0), (1, 0), (1, 2), (2, 0), (2, 1)]
        } else {
            &[(0, 0), (1, 1), (2, 0), (3, 0)]
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
        // Three dispersed samples OR a single plain sample. Four static sites, not four executed samples.
        assert_eq!(
            operations
                .iter()
                .filter(|op| **op == Op::ImageSampleExplicitLod)
                .count(),
            4
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

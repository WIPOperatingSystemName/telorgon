//! Compiled backend-neutral material and render-pass contracts.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::ui::MaterialId;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaterialPassKind {
    Capture,
    HorizontalBlur,
    VerticalBlur,
    Tint,
    Shadow,
    Composite,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MaterialPass {
    pub kind: MaterialPassKind,
    pub radius: f32,
    pub color: ColorRgba8,
    pub input: u8,
    pub output: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialContract {
    pub id: MaterialId,
    pub passes: Arc<[MaterialPass]>,
    pub damage_expansion: f32,
    pub intermediate_targets: u8,
}

#[derive(Clone, Debug, Default)]
pub struct MaterialLibrary {
    contracts: Vec<MaterialContract>,
    names: BTreeMap<String, MaterialId>,
}
impl MaterialLibrary {
    pub fn register(
        &mut self,
        name: impl Into<String>,
        passes: impl Into<Arc<[MaterialPass]>>,
    ) -> MaterialId {
        let name = name.into();
        if let Some(id) = self.names.get(&name).copied() {
            return id;
        }
        let passes = passes.into();
        let damage_expansion = passes.iter().map(|pass| pass.radius).fold(0.0, f32::max);
        let intermediate_targets = passes.iter().map(|pass| pass.output).max().unwrap_or(0);
        let id = MaterialId(self.contracts.len() as u32);
        self.contracts.push(MaterialContract {
            id,
            passes,
            damage_expansion,
            intermediate_targets,
        });
        self.names.insert(name, id);
        id
    }
    pub fn id(&self, name: &str) -> Option<MaterialId> {
        self.names.get(name).copied()
    }
    pub fn get(&self, id: MaterialId) -> Option<&MaterialContract> {
        self.contracts.get(id.0 as usize)
    }
    pub fn builtins() -> Self {
        let mut library = Self::default();
        library.register(
            "shadow",
            Arc::from([
                MaterialPass {
                    kind: MaterialPassKind::Shadow,
                    radius: 12.0,
                    color: ColorRgba8::rgba(0, 0, 0, 128),
                    input: 0,
                    output: 1,
                },
                MaterialPass {
                    kind: MaterialPassKind::Composite,
                    radius: 0.0,
                    color: ColorRgba8::default(),
                    input: 1,
                    output: 0,
                },
            ]),
        );
        library.register(
            "glass",
            Arc::from([
                MaterialPass {
                    kind: MaterialPassKind::Capture,
                    radius: 0.0,
                    color: ColorRgba8::default(),
                    input: 0,
                    output: 1,
                },
                MaterialPass {
                    kind: MaterialPassKind::HorizontalBlur,
                    radius: 8.0,
                    color: ColorRgba8::default(),
                    input: 1,
                    output: 2,
                },
                MaterialPass {
                    kind: MaterialPassKind::VerticalBlur,
                    radius: 8.0,
                    color: ColorRgba8::default(),
                    input: 2,
                    output: 1,
                },
                MaterialPass {
                    kind: MaterialPassKind::Tint,
                    radius: 0.0,
                    color: ColorRgba8::rgba(255, 255, 255, 32),
                    input: 1,
                    output: 0,
                },
            ]),
        );
        library
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn materials_are_compiled_once_and_borrowed() {
        let library = MaterialLibrary::builtins();
        let id = library.id("glass").unwrap();
        assert_eq!(library.get(id).unwrap().passes.len(), 4);
    }
}

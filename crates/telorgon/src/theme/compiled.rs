//! Immutable, catalog-validated Theme v4 tables.

use std::collections::BTreeMap;

use crate::ui::{
    ComponentStyleId, InteractionFlags, StylePropertyPatch, StyleSlotId, StyleVariantSelection,
    VariantAxisId, VariantValueId,
};

use crate::theme::{InteractionState, ThemeDiagnostic, ThemeDomain, TransitionSpec};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompiledSlotStyle {
    pub patch: StylePropertyPatch,
    pub font_family: Option<String>,
}

impl CompiledSlotStyle {
    pub fn overlay(&mut self, other: &Self) {
        self.patch.overlay(other.patch);
        if other.font_family.is_some() {
            self.font_family.clone_from(&other.font_family);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompiledStateStyle {
    pub slots: BTreeMap<StyleSlotId, CompiledSlotStyle>,
    pub transition: Option<TransitionSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledComponentStyle {
    pub id: ComponentStyleId,
    pub slots: BTreeMap<StyleSlotId, CompiledSlotStyle>,
    pub variants:
        BTreeMap<(VariantAxisId, VariantValueId), BTreeMap<StyleSlotId, CompiledSlotStyle>>,
    pub states: BTreeMap<InteractionState, CompiledStateStyle>,
    pub state_precedence: Vec<InteractionState>,
    pub relevant_states: InteractionFlags,
    pub transition: TransitionSpec,
    /// Union of properties owned by this contract across base, variant, and state layers.
    pub controlled_slots: BTreeMap<StyleSlotId, StylePropertyPatch>,
    pub controlled_font_families: std::collections::BTreeSet<StyleSlotId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedComponentStyle {
    pub slots: BTreeMap<StyleSlotId, CompiledSlotStyle>,
    pub transition: TransitionSpec,
    pub controlled_slots: BTreeMap<StyleSlotId, StylePropertyPatch>,
    pub controlled_font_families: std::collections::BTreeSet<StyleSlotId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedSlotStyle<'a> {
    pub patch: StylePropertyPatch,
    pub font_family: Option<&'a str>,
    pub transition: TransitionSpec,
}

impl CompiledComponentStyle {
    /// Merge an authored effect layer without losing theme focus/pressed/disabled states.
    pub(crate) fn overlay_effects(&mut self, effects: &Self) {
        for (slot, value) in &effects.slots { self.slots.entry(*slot).or_default().overlay(value); }
        for (state, value) in &effects.states {
            let target = self.states.entry(*state).or_default();
            for (slot, patch) in &value.slots { target.slots.entry(*slot).or_default().overlay(patch); }
            if value.transition.is_some() { target.transition = value.transition; }
        }
        for (slot, patch) in &effects.controlled_slots { self.controlled_slots.entry(*slot).or_default().overlay(*patch); }
        self.controlled_font_families.extend(effects.controlled_font_families.iter().copied());
        self.relevant_states = InteractionFlags::from_bits(self.relevant_states.bits() | effects.relevant_states.bits());
        self.state_precedence.retain(|state| !matches!(state, InteractionState::Hovered | InteractionState::Pressed));
        self.state_precedence.insert(0, InteractionState::Pressed);
        self.state_precedence.insert(0, InteractionState::Hovered);
        self.transition = effects.transition;
    }

    /// Resolves one mounted slot without allocating. The owned `resolve` form remains available
    /// for diagnostics and tooling, while the frame pipeline uses this path.
    pub fn resolve_slot<'a>(
        &'a self,
        variants: &[StyleVariantSelection],
        flags: InteractionFlags,
        slot: StyleSlotId,
    ) -> Option<ResolvedSlotStyle<'a>> {
        let mut patch = StylePropertyPatch::default();
        let mut font_family = None;
        let mut present = false;
        let mut overlay = |style: &'a CompiledSlotStyle| {
            present = true;
            patch.overlay(style.patch);
            if let Some(family) = style.font_family.as_deref() {
                font_family = Some(family);
            }
        };
        if let Some(style) = self.slots.get(&slot) {
            overlay(style);
        }
        for variant in variants {
            if let Some(style) = self
                .variants
                .get(&(variant.axis, variant.value))
                .and_then(|slots| slots.get(&slot))
            {
                overlay(style);
            }
        }
        let flags = InteractionFlags::from_bits(flags.bits() & self.relevant_states.bits());
        let mut transition = self.transition;
        for state in &self.state_precedence {
            if !flags.contains(state.flag()) || (matches!(state, InteractionState::Hovered | InteractionState::Pressed) && flags.contains(InteractionFlags::DISABLED)) {
                continue;
            }
            if let Some(state_style) = self.states.get(state) {
                if let Some(style) = state_style.slots.get(&slot) {
                    overlay(style);
                }
                if let Some(spec) = state_style.transition {
                    transition = spec;
                }
            }
        }
        present.then_some(ResolvedSlotStyle {
            patch,
            font_family,
            transition,
        })
    }

    pub fn resolve(
        &self,
        variants: &[StyleVariantSelection],
        flags: InteractionFlags,
    ) -> ResolvedComponentStyle {
        let mut resolved = ResolvedComponentStyle {
            slots: self.slots.clone(),
            transition: self.transition,
            controlled_slots: self.controlled_slots.clone(),
            controlled_font_families: self.controlled_font_families.clone(),
        };
        for variant in variants {
            if let Some(slots) = self.variants.get(&(variant.axis, variant.value)) {
                overlay_slots(&mut resolved.slots, slots);
            }
        }
        let flags = InteractionFlags::from_bits(flags.bits() & self.relevant_states.bits());
        for state in &self.state_precedence {
            if !flags.contains(state.flag()) || (matches!(state, InteractionState::Hovered | InteractionState::Pressed) && flags.contains(InteractionFlags::DISABLED)) {
                continue;
            }
            if let Some(overlay) = self.states.get(state) {
                overlay_slots(&mut resolved.slots, &overlay.slots);
                if let Some(transition) = overlay.transition {
                    resolved.transition = transition;
                }
            }
        }
        resolved
    }
}

fn overlay_slots(
    target: &mut BTreeMap<StyleSlotId, CompiledSlotStyle>,
    overlay: &BTreeMap<StyleSlotId, CompiledSlotStyle>,
) {
    for (slot, style) in overlay {
        target.entry(*slot).or_default().overlay(style);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledTheme {
    pub(crate) domain: ThemeDomain,
    pub(crate) styles: BTreeMap<ComponentStyleId, CompiledComponentStyle>,
    pub(crate) names: BTreeMap<String, ComponentStyleId>,
    pub(crate) diagnostics: Vec<ThemeDiagnostic>,
    pub(crate) fingerprint: u64,
}

impl CompiledTheme {
    pub const fn domain(&self) -> ThemeDomain {
        self.domain
    }

    pub fn style_id(&self, component: &str, style: &str) -> Option<ComponentStyleId> {
        self.names.get(&format!("{component}.{style}")).copied()
    }

    pub fn style(&self, id: ComponentStyleId) -> Option<&CompiledComponentStyle> {
        self.styles.get(&id)
    }

    pub fn diagnostics(&self) -> &[ThemeDiagnostic] {
        &self.diagnostics
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }

    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub fn changed_style_ids(&self, replacement: &Self) -> Vec<ComponentStyleId> {
        self.styles
            .keys()
            .chain(replacement.styles.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|id| self.styles.get(id) != replacement.styles.get(id))
            .collect()
    }
}

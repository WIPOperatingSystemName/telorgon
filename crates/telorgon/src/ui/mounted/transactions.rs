use super::*;

impl MountedUi {
    #[cfg(test)]
    pub(super) fn insert_fixture(
        &mut self,
        parent: Option<NodeId>,
        before: Option<NodeId>,
        fixture: NodeFixture,
    ) -> Option<NodeId> {
        if before.is_some() && parent.is_none() {
            return None;
        }
        let node = self.nodes.spawn(parent)?;
        if before.is_some() && !self.nodes.reparent_before(node, parent?, before) {
            self.nodes.remove_subtree(node);
            return None;
        }
        self.kinds.insert(node, fixture.kind);
        if fixture.style != BoxStyle::default() {
            self.box_styles.insert(node, fixture.style);
        }
        if fixture.layout != LayoutStyle::default() {
            self.layouts.insert(node, fixture.layout);
        }
        if fixture.interaction != InteractionSnapshot::default() {
            self.interactions.insert(node, fixture.interaction);
            if let Some(core) = self.nodes.core_mut(node) {
                core.state_bits = fixture.interaction.flags.bits();
            }
        }
        if fixture.key.is_some() {
            self.keys.insert(node, fixture.key);
        }
        if let Some(text) = fixture.text {
            self.texts.insert(node, text);
        }
        if let Some(image) = fixture.image {
            self.images.insert(node, image);
        }
        if let Some(semantic) = fixture.semantic {
            self.semantics.insert(node, semantic);
        }
        for child in fixture.children {
            self.insert_fixture(Some(node), None, child);
        }
        Some(node)
    }

    pub(super) fn commit(&mut self) -> TransactionResult {
        let mut result = TransactionResult::default();
        let patches = std::mem::take(&mut self.patch_log);
        for patch in &patches {
            if self.apply_patch(patch) {
                result.property_patches += 1;
            }
        }
        self.patch_log = patches;
        self.patch_log.clear();
        let mut structural = std::mem::take(&mut self.structural_log);
        for command in structural.drain(..) {
            match command {
                StructuralCommand::Remove(node) => {
                    if !self.remove(node).is_empty() {
                        result.structural_mutations += 1;
                    }
                }
                #[cfg(test)]
                StructuralCommand::Reconcile { parent, children } => {
                    result.structural_mutations += self.reconcile(parent, children);
                }
            }
        }
        self.structural_log = structural;
        self.diagnostics.property_patches += result.property_patches as u64;
        self.diagnostics.structural_mutations += result.structural_mutations as u64;
        result
    }

    #[cfg(test)]
    pub(super) fn reconcile(&mut self, parent: NodeId, children: Vec<NodeFixture>) -> usize {
        if !self.nodes.contains(parent) {
            return 0;
        }
        let existing: Vec<_> = self.nodes.children(parent).collect();
        let mut retained = Vec::with_capacity(children.len());
        let mut before = existing.first().copied();
        let mut mutations = 0;
        for fixture in children {
            let reusable = fixture.key.and_then(|key| {
                existing.iter().copied().find(|node| {
                    !retained.contains(node) && self.keys.get(*node).copied().flatten() == Some(key)
                })
            });
            if let Some(node) = reusable {
                if before == Some(node) {
                    before = self.nodes.core(node).and_then(|core| core.next_sibling);
                } else if self.nodes.reparent_before(node, parent, before) {
                    mutations += 1;
                }
                retained.push(node);
                mutations += self.sync_fixture(node, fixture);
            } else if let Some(node) = self.insert_fixture(Some(parent), before, fixture) {
                retained.push(node);
                mutations += 1;
            }
        }
        for node in existing {
            if !retained.contains(&node) {
                self.remove(node);
                mutations += 1;
            }
        }
        mutations
    }

    #[cfg(test)]
    pub(super) fn sync_fixture(&mut self, node: NodeId, fixture: NodeFixture) -> usize {
        let NodeFixture {
            key,
            kind,
            style,
            layout,
            interaction,
            text,
            image,
            semantic,
            children,
        } = fixture;
        let kind_changed = replace_value(&mut self.kinds, node, kind);
        let style_changed = replace_default(&mut self.box_styles, node, style);
        let layout_changed = replace_default(&mut self.layouts, node, layout);
        let interaction_changed = replace_default(&mut self.interactions, node, interaction);
        let text_changed = replace_optional(&mut self.texts, node, text);
        let image_changed = replace_optional(&mut self.images, node, image);
        let semantic_changed = replace_optional(&mut self.semantics, node, semantic);
        replace_optional(&mut self.keys, node, key.map(Some));
        let mut dirty = DirtyFlags::NONE;
        if kind_changed || style_changed {
            dirty |= DirtyFlags::STYLE
                | DirtyFlags::LAYOUT
                | DirtyFlags::SPATIAL
                | DirtyFlags::CLIP
                | DirtyFlags::PAINT;
        }
        if layout_changed {
            dirty |= DirtyFlags::LAYOUT | DirtyFlags::SPATIAL | DirtyFlags::CLIP;
        }
        if interaction_changed {
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS;
        }
        if text_changed {
            dirty |= DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT;
        }
        if image_changed {
            dirty |= DirtyFlags::PAINT;
        }
        if semantic_changed {
            dirty |= DirtyFlags::SEMANTICS;
        }
        if dirty != DirtyFlags::NONE {
            self.nodes.mark_dirty(node, dirty);
            if let Some(core) = self.nodes.core_mut(node) {
                core.style_revision += u64::from(style_changed || interaction_changed);
                core.content_revision += u64::from(text_changed || image_changed);
                core.semantic_revision += u64::from(semantic_changed);
                core.state_bits = interaction.flags.bits();
            }
        }
        usize::from(dirty != DirtyFlags::NONE) + self.reconcile(node, children)
    }

    pub(super) fn apply_patch(&mut self, patch: &Patch) -> bool {
        if !self.nodes.contains(patch.node) {
            return false;
        }
        match (&patch.kind, &patch.value) {
            (PropertyKind::Enabled, PropertyValue::Bool(false))
            | (PropertyKind::Visible, PropertyValue::Bool(false)) => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            (PropertyKind::Value, PropertyValue::Float(value)) if *value != 0.0 => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            (PropertyKind::Opacity, PropertyValue::Float(value)) if *value != 1.0 => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Background, PropertyValue::Color(_)) => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Translation, PropertyValue::Point(value))
                if *value != PointF::default() =>
            {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::ScrollOffset, PropertyValue::Point(value))
                if *value != PointF::default() =>
            {
                if self.layouts.get(patch.node).is_none() {
                    self.layouts.insert(patch.node, LayoutStyle::default());
                }
            }
            (PropertyKind::Style, PropertyValue::Style(value)) if *value != BoxStyle::default() => {
                if self.box_styles.get(patch.node).is_none() {
                    self.box_styles.insert(patch.node, BoxStyle::default());
                }
            }
            (PropertyKind::Checked, PropertyValue::Check(_))
            | (PropertyKind::Busy, PropertyValue::Bool(_)) => {
                if self.interactions.get(patch.node).is_none() {
                    self.interactions
                        .insert(patch.node, InteractionSnapshot::default());
                }
            }
            _ => {}
        }
        let changed = match (&patch.kind, &patch.value) {
            (PropertyKind::Enabled, PropertyValue::Bool(value)) => self
                .interactions
                .get_mut(patch.node)
                .is_some_and(|interaction| interaction.set_enabled(*value)),
            (PropertyKind::Visible, PropertyValue::Bool(value)) => change_interaction(
                &mut self.interactions,
                patch.node,
                |item| &mut item.visible,
                *value,
            ),
            (PropertyKind::Value, PropertyValue::Float(value)) => {
                let interaction_changed = change_interaction(
                    &mut self.interactions,
                    patch.node,
                    |item| &mut item.value,
                    *value,
                );
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    let SemanticValue::Number { current, .. } = &mut semantic.value else {
                        return false;
                    };
                    let value = f64::from(*value);
                    if *current == value {
                        false
                    } else {
                        *current = value;
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            (PropertyKind::Opacity, PropertyValue::Float(value)) => change_style(
                &mut self.box_styles,
                patch.node,
                |style| &mut style.opacity,
                value.clamp(0.0, 1.0),
            ),
            (PropertyKind::Text, PropertyValue::String(value)) => {
                self.texts.get_mut(patch.node).is_some_and(|text| {
                    if text.content == *value {
                        false
                    } else {
                        text.content = *value;
                        text.revision += 1;
                        true
                    }
                })
            }
            (PropertyKind::TextColor, PropertyValue::Color(value)) => {
                self.texts.get_mut(patch.node).is_some_and(|text| {
                    if text.style.color == *value {
                        false
                    } else {
                        text.style.color = *value;
                        true
                    }
                })
            }
            (PropertyKind::Background, PropertyValue::Color(value)) => change_style(
                &mut self.box_styles,
                patch.node,
                |style| &mut style.decoration.background,
                Background::Color(*value),
            ),
            (PropertyKind::Translation, PropertyValue::Point(value)) => {
                self.box_styles.get_mut(patch.node).is_some_and(|style| {
                    if style.transform.translation == *value {
                        false
                    } else {
                        style.transform.translation = *value;
                        true
                    }
                })
            }
            (PropertyKind::ScrollOffset, PropertyValue::Point(value)) => change_layout(
                &mut self.layouts,
                patch.node,
                |layout| &mut layout.scroll_offset,
                *value,
            ),
            (PropertyKind::Style, PropertyValue::Style(value)) => {
                self.box_styles.get_mut(patch.node).is_some_and(|style| {
                    if style == value {
                        false
                    } else {
                        *style = *value;
                        true
                    }
                })
            }
            (PropertyKind::Checked, PropertyValue::Check(value)) => {
                let interaction_changed =
                    self.interactions
                        .get_mut(patch.node)
                        .is_some_and(|interaction| {
                            let checked = *value != SemanticCheckState::Unchecked;
                            let mixed = *value == SemanticCheckState::Mixed;
                            let checked_changed =
                                interaction.set_flag(InteractionFlags::CHECKED, checked);
                            let mixed_changed =
                                interaction.set_flag(InteractionFlags::MIXED, mixed);
                            checked_changed || mixed_changed
                        });
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    if semantic.state.checked == Some(*value) {
                        false
                    } else {
                        semantic.state.checked = Some(*value);
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            (PropertyKind::Busy, PropertyValue::Bool(value)) => {
                let interaction_changed =
                    self.interactions
                        .get_mut(patch.node)
                        .is_some_and(|interaction| {
                            interaction.set_flag(InteractionFlags::BUSY, *value)
                        });
                let semantic_changed = self.semantics.get_mut(patch.node).is_some_and(|semantic| {
                    if semantic.state.busy == *value {
                        false
                    } else {
                        semantic.state.busy = *value;
                        true
                    }
                });
                interaction_changed || semantic_changed
            }
            _ => false,
        };
        if changed {
            if patch.kind == PropertyKind::Enabled
                && let Some(interaction) = self.interactions.get_mut(patch.node)
            {
                if let Some(core) = self.nodes.core_mut(patch.node) {
                    core.state_bits = interaction.flags.bits();
                }
            }
            if matches!(patch.kind, PropertyKind::Checked | PropertyKind::Busy) {
                if let Some(interaction) = self.interactions.get(patch.node)
                    && let Some(core) = self.nodes.core_mut(patch.node)
                {
                    core.state_bits = interaction.flags.bits();
                }
                if let Some(core) = self.nodes.core_mut(patch.node) {
                    core.semantic_revision += 1;
                }
            }
            if patch.kind == PropertyKind::Value
                && let Some(core) = self.nodes.core_mut(patch.node)
            {
                core.semantic_revision += 1;
            }
            if matches!(
                patch.kind,
                PropertyKind::Enabled
                    | PropertyKind::Visible
                    | PropertyKind::Value
                    | PropertyKind::Checked
                    | PropertyKind::Busy
            ) && let Some(core) = self.nodes.core_mut(patch.node)
            {
                core.style_revision = core.style_revision.wrapping_add(1).max(1);
                if let Some(interaction) = self.interactions.get(patch.node) {
                    core.state_bits = interaction.flags.bits();
                }
            }
            let dirty = match patch.kind {
                PropertyKind::Text => DirtyFlags::TEXT | DirtyFlags::MEASURE | DirtyFlags::PAINT,
                PropertyKind::ScrollOffset | PropertyKind::Translation => {
                    DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT
                }
                PropertyKind::Visible => {
                    DirtyFlags::VISIBILITY
                        | DirtyFlags::SPATIAL
                        | DirtyFlags::CLIP
                        | DirtyFlags::PAINT
                }
                PropertyKind::Enabled
                | PropertyKind::Opacity
                | PropertyKind::TextColor
                | PropertyKind::Background => DirtyFlags::STYLE | DirtyFlags::PAINT,
                PropertyKind::Value => {
                    DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS
                }
                PropertyKind::Checked | PropertyKind::Busy => {
                    DirtyFlags::STYLE | DirtyFlags::PAINT | DirtyFlags::SEMANTICS
                }
                PropertyKind::Style => {
                    DirtyFlags::STYLE
                        | DirtyFlags::LAYOUT
                        | DirtyFlags::SPATIAL
                        | DirtyFlags::CLIP
                        | DirtyFlags::PAINT
                }
            };
            self.nodes.mark_dirty(patch.node, dirty);
            if matches!(
                patch.kind,
                PropertyKind::Enabled
                    | PropertyKind::Visible
                    | PropertyKind::Value
                    | PropertyKind::Checked
                    | PropertyKind::Busy
            ) {
                self.enqueue_style_bindings_for_state(patch.node);
            }
        }
        changed
    }
}

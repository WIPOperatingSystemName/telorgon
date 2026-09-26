use super::*;

impl MountedUi {
    /// Replaces one authored box style while retaining the node and its interaction state.
    pub fn set_box_style(&mut self, node: NodeId, style: BoxStyle) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let current = self.box_styles.get(node).copied().unwrap_or_default();
        if current == style {
            return false;
        }
        if style == BoxStyle::default() {
            self.box_styles.remove(node);
        } else {
            self.box_styles.insert(node, style);
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::STYLE
                | DirtyFlags::LAYOUT
                | DirtyFlags::SPATIAL
                | DirtyFlags::CLIP
                | DirtyFlags::PAINT,
        );
        true
    }

    /// Replaces authored layout inputs without reconstructing the retained node.
    pub fn set_layout_style(&mut self, node: NodeId, layout: LayoutStyle) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let current = self.layouts.get(node).copied().unwrap_or_default();
        if current == layout {
            return false;
        }
        if layout == LayoutStyle::default() {
            self.layouts.remove(node);
        } else {
            self.layouts.insert(node, layout);
        }
        self.nodes.mark_dirty(
            node,
            DirtyFlags::LAYOUT | DirtyFlags::SPATIAL | DirtyFlags::CLIP | DirtyFlags::PAINT,
        );
        true
    }

    /// Replaces retained text styling while preserving content storage and semantic identity.
    pub fn set_text_style(&mut self, node: NodeId, style: TextStyle) -> bool {
        let Some(text) = self.texts.get_mut(node) else {
            return false;
        };
        if text.style == style {
            return false;
        }
        let metrics_changed = text.style.size != style.size
            || text.style.line_height != style.line_height
            || text.style.family != style.family || text.style.weight != style.weight
            || text.style.fit_height != style.fit_height;
        let alignment_changed = text.style.align != style.align || text.style.vertical_align != style.vertical_align;
        text.style = style;
        let mut dirty = DirtyFlags::PAINT;
        if metrics_changed {
            text.revision = text.revision.wrapping_add(1).max(1);
            dirty |= DirtyFlags::TEXT | DirtyFlags::LAYOUT;
        }
        if alignment_changed { dirty |= DirtyFlags::LAYOUT; }
        if let Some(core) = self.nodes.core_mut(node) {
            core.content_revision = core.content_revision.wrapping_add(1).max(1);
        }
        self.nodes.mark_dirty(node, dirty);
        true
    }

    pub fn style_bindings(&self) -> &[StyleBinding] {
        &self.style_bindings
    }

    /// Registers one complete binding only when its state root and every named slot are live.
    pub fn register_style_binding(&mut self, binding: StyleBinding) -> bool {
        let state_root = binding.state_root;
        if !self.nodes.contains(binding.state_root)
            || binding.slots.is_empty()
            || binding
                .slots
                .iter()
                .any(|slot| !self.nodes.contains(slot.node))
        {
            return false;
        }
        if self.style_bindings.contains(&binding) {
            return false;
        }
        // Promote the foundation root binding to the complete component contract.
        // Keeping both creates competing tracks for the same root slot, and later
        // reconciliation updates only the first binding, leaving stale inline state.
        let root_slot = StyleSlotBinding {
            slot: StyleSlotId::named("root"),
            node: state_root,
        };
        if binding.slots.contains(&root_slot)
            && let Some(index) = self.style_bindings.iter().position(|existing| {
                existing.state_root == state_root
                    && existing.scope == binding.scope
                    && existing.component_style == binding.component_style
                    && existing.slots.as_slice() == [root_slot]
                    && existing.local_style.is_none()
                    && existing.local_overrides.is_empty()
                    && existing.variants.is_empty()
            })
        {
            self.style_bindings[index] = binding;
            self.enqueue_style_binding(index);
            if let Some(core) = self.nodes.core_mut(state_root) {
                core.style_revision = core.style_revision.wrapping_add(1).max(1);
            }
            return true;
        }
        let index = self.style_bindings.len();
        let next = self.style_binding_head_by_state.get(state_root).copied();
        self.style_bindings.push(binding);
        self.style_binding_next.push(next);
        self.style_binding_head_by_state.insert(state_root, index);
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(state_root) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        true
    }

    /// Selects a stable style for the automatically registered root binding of a component.
    pub fn set_style_id(&mut self, node: NodeId, style: ComponentStyleId) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding
                    .slots
                    .iter()
                    .any(|slot| slot.node == node && slot.slot == StyleSlotId::named("root"))
        }) else {
            return false;
        };
        if self.style_bindings[index].component_style == style {
            return false;
        }
        let binding = &mut self.style_bindings[index];
        binding.component_style = style;
        binding.theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    /// Installs one sparse local slot override without bypassing theme state resolution.
    pub fn set_style_override(
        &mut self,
        node: NodeId,
        slot: StyleSlotId,
        patch: StylePropertyPatch,
    ) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding.slots.iter().any(|candidate| candidate.node == node)
        }) else {
            return false;
        };
        let binding = &mut self.style_bindings[index];
        if let Some((_, existing)) = binding
            .local_overrides
            .iter_mut()
            .find(|(candidate, _)| *candidate == slot)
        {
            if *existing == patch {
                return false;
            }
            *existing = patch;
        } else {
            binding.local_overrides.push((slot, patch));
        }
        binding.theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    /// Replaces an optional code-defined component style on an existing binding.
    #[doc(hidden)]
    pub fn set_local_component_style(
        &mut self,
        node: NodeId,
        style: Option<Arc<crate::theme::CompiledComponentStyle>>,
    ) -> bool {
        let Some(index) = self.style_bindings.iter().position(|binding| {
            binding.state_root == node
                && binding
                    .slots
                    .iter()
                    .any(|slot| slot.node == node && slot.slot == StyleSlotId::named("root"))
        }) else {
            return false;
        };
        if self.style_bindings[index].local_style == style {
            return false;
        }
        self.style_bindings[index].local_style = style;
        self.style_bindings[index].theme_revision = 0;
        self.enqueue_style_binding(index);
        if let Some(core) = self.nodes.core_mut(node) {
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        true
    }

    pub(crate) fn set_local_style_overlay(&mut self, node: NodeId, overlay: bool) {
        let Some(index) = self.style_bindings.iter().position(|binding| binding.state_root == node && binding.slots.iter().any(|slot| slot.node == node && slot.slot == StyleSlotId::named("root"))) else { return; };
        if self.style_bindings[index].local_style_overlay != overlay {
            self.style_bindings[index].local_style_overlay = overlay;
            self.style_bindings[index].theme_revision = 0;
            self.enqueue_style_binding(index);
        }
    }

    pub(crate) fn reset_local_style_motion(&mut self, node: NodeId) {
        if let Some(index) = self.style_bindings.iter().position(|binding| binding.state_root == node) {
            self.style_bindings[index].reset_style_motion = true;
            self.style_bindings[index].theme_revision = 0;
            self.enqueue_style_binding(index);
        }
    }

    /// Requalifies automatically mounted foundation bindings for an application or shell view.
    pub fn set_theme_domain(&mut self, domain: ThemeDomainId, scope: ThemeScopeId) {
        for (index, binding) in self.style_bindings.iter_mut().enumerate() {
            binding.scope = scope;
            binding.component_style.domain = domain;
            binding.theme_revision = 0;
            if self.dirty_style_binding_marks.len() <= index {
                self.dirty_style_binding_marks.resize(index + 1, false);
            }
            if !self.dirty_style_binding_marks[index] {
                self.dirty_style_binding_marks[index] = true;
                self.dirty_style_bindings.push(index);
            }
        }
        let nodes = self.nodes.alive().to_vec();
        for node in nodes {
            self.nodes
                .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        }
    }

    pub(super) fn enqueue_style_binding(&mut self, index: usize) {
        if self.dirty_style_binding_marks.len() <= index {
            self.dirty_style_binding_marks.resize(index + 1, false);
        }
        if !self.dirty_style_binding_marks[index] {
            self.dirty_style_binding_marks[index] = true;
            self.dirty_style_bindings.push(index);
        }
    }

    pub(super) fn enqueue_style_bindings_for_state(&mut self, state_root: NodeId) {
        let mut cursor = self.style_binding_head_by_state.get(state_root).copied();
        while let Some(index) = cursor {
            self.enqueue_style_binding(index);
            cursor = self.style_binding_next.get(index).copied().flatten();
        }
    }

    pub(super) fn rebuild_style_binding_index(&mut self) {
        self.style_binding_head_by_state = SparseSet::default();
        self.style_binding_next.clear();
        self.dirty_style_bindings.clear();
        self.dirty_style_binding_marks.clear();
        for (index, binding) in self.style_bindings.iter().enumerate() {
            let next = self
                .style_binding_head_by_state
                .get(binding.state_root)
                .copied();
            self.style_binding_next.push(next);
            self.style_binding_head_by_state
                .insert(binding.state_root, index);
            self.dirty_style_bindings.push(index);
            self.dirty_style_binding_marks.push(true);
        }
    }

    #[doc(hidden)]
    pub fn take_style_bindings_for_processing(&mut self) -> Vec<StyleBinding> {
        std::mem::take(&mut self.style_bindings)
    }

    #[doc(hidden)]
    pub fn restore_style_bindings_after_processing(&mut self, bindings: Vec<StyleBinding>) {
        debug_assert!(self.style_bindings.is_empty());
        self.style_bindings = bindings;
    }

    #[doc(hidden)]
    pub fn swap_dirty_style_bindings(&mut self, scratch: &mut Vec<usize>) {
        std::mem::swap(&mut self.dirty_style_bindings, scratch);
        for &index in scratch.iter() {
            if let Some(mark) = self.dirty_style_binding_marks.get_mut(index) {
                *mark = false;
            }
        }
    }

    /// Applies one already-resolved slot patch as a single dirty-state publication.
    pub fn apply_style_patch(&mut self, node: NodeId, patch: StylePropertyPatch) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        let style = self.box_styles.get(node).copied().unwrap_or_default();
        let mut next = style;
        macro_rules! assign {
            ($($field:ident),+ $(,)?) => {$ (
                if let Some(value) = patch.$field {
                    next.$field = value;
                }
            )+ };
        }
        assign!(
            sizing, width, height, min_size, max_size, margin, padding, overflow, opacity,
            transform,
        );
        if let Some(value) = patch.background {
            next.decoration.background = value;
        }
        if let Some(value) = patch.border {
            next.decoration.border = value;
        }
        if let Some(value) = patch.outline {
            next.decoration.outline = value;
        }
        if let Some(value) = patch.corner_radii {
            next.decoration.corner_radii = value;
        }
        if let Some(value) = patch.shadows {
            next.decoration.shadows = value;
        }
        if let Some(width) = patch.border_width {
            next.decoration.border.top.width = width;
            next.decoration.border.right.width = width;
            next.decoration.border.bottom.width = width;
            next.decoration.border.left.width = width;
        }
        if let Some(color) = patch.border_color {
            next.decoration.border.top.color = color;
            next.decoration.border.right.color = color;
            next.decoration.border.bottom.color = color;
            next.decoration.border.left.color = color;
        }
        if let Some(width) = patch.outline_width {
            next.decoration.outline.width = width;
        }
        if let Some(offset) = patch.outline_offset {
            next.decoration.outline.offset = offset;
        }
        if let Some(color) = patch.outline_color {
            next.decoration.outline.color = color;
        }
        if let Some(radius) = patch.radius {
            next.decoration.corner_radii = CornerRadii::all(radius);
        }
        if let Some(value) = patch.translation_x {
            next.transform.translation.x = value;
        }
        if let Some(value) = patch.translation_y {
            next.transform.translation.y = value;
        }
        if let Some(value) = patch.scale_x {
            next.transform.scale.x = value;
        }
        if let Some(value) = patch.scale_y {
            next.transform.scale.y = value;
        }
        if let Some(value) = patch.rotation {
            next.transform.rotation = value;
        }
        if let Some(value) = patch.origin_x {
            next.transform.origin.x = value;
        }
        if let Some(value) = patch.origin_y {
            next.transform.origin.y = value;
        }
        let mut dirty = DirtyFlags::NONE;
        if next != style {
            if next.sizing != style.sizing
                || next.width != style.width
                || next.height != style.height
                || next.min_size != style.min_size
                || next.max_size != style.max_size
                || next.margin != style.margin
                || next.padding != style.padding
                || next.decoration.border.top.width != style.decoration.border.top.width
                || next.decoration.border.right.width != style.decoration.border.right.width
                || next.decoration.border.bottom.width != style.decoration.border.bottom.width
                || next.decoration.border.left.width != style.decoration.border.left.width
            {
                dirty |= DirtyFlags::LAYOUT;
            }
            if next.transform != style.transform || next.overflow != style.overflow {
                dirty |= DirtyFlags::SPATIAL | DirtyFlags::CLIP;
            }
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
            self.box_styles.insert(node, next);
        }
        if let Some(text) = self.texts.get_mut(node) {
            let old = text.style;
            if let Some(value) = patch.text_color {
                text.style.color = value;
            }
            if let Some(value) = patch.text_size {
                text.style.size = value;
            }
            if let Some(value) = patch.text_line_height {
                text.style.line_height = value;
            }
            if let Some(value) = patch.text_family {
                text.style.family = value;
            }
            if let Some(value) = patch.text_weight {
                text.style.weight = value;
            }
            if text.style != old {
                dirty |= DirtyFlags::PAINT;
                if text.style.size != old.size
                    || text.style.line_height != old.line_height
                    || text.style.family != old.family
                    || text.style.weight != old.weight
                {
                    text.revision = text.revision.wrapping_add(1).max(1);
                    dirty |= DirtyFlags::TEXT | DirtyFlags::LAYOUT;
                }
            }
        }
        if let Some(image) = self.images.get_mut(node)
            && let Some(tint) = patch.image_tint
            && image.tint != tint
        {
            image.tint = tint;
            dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
        }
        if dirty == DirtyFlags::NONE {
            return false;
        }
        self.nodes.mark_dirty(node, dirty);
        true
    }
}

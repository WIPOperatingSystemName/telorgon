use super::*;

impl MountedUi {
    pub(super) fn set_interaction_flag(
        &mut self,
        node: NodeId,
        flag: InteractionFlags,
        enabled: bool,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if !enabled {
                return false;
            }
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        let changed = if flag == InteractionFlags::DISABLED {
            interaction.set_enabled(!enabled)
        } else {
            interaction.set_flag(flag, enabled)
        };
        if !changed {
            return false;
        }
        if let Some(core) = self.nodes.core_mut(node) {
            core.state_bits = interaction.flags.bits();
            core.style_revision = core.style_revision.wrapping_add(1).max(1);
        }
        self.nodes
            .mark_dirty(node, DirtyFlags::STYLE | DirtyFlags::PAINT);
        self.enqueue_style_bindings_for_state(node);
        true
    }

    /// Publishes router-owned transient/focus state. This is intentionally separated from
    /// component-controlled semantic state so controls cannot impersonate the input router.
    #[doc(hidden)]
    pub fn route_interaction_flag(
        &mut self,
        node: NodeId,
        flag: InteractionFlags,
        enabled: bool,
    ) -> bool {
        if flag.bits() & !InteractionFlags::ROUTER_OWNED.bits() != 0
            || flag.bits().count_ones() != 1
        {
            return false;
        }
        self.set_interaction_flag(node, flag, enabled)
    }

    pub fn set_disabled(&mut self, node: NodeId, disabled: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::DISABLED, disabled)
    }

    pub fn set_read_only(&mut self, node: NodeId, read_only: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::READ_ONLY, read_only)
    }

    pub fn set_busy(&mut self, node: NodeId, busy: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::BUSY, busy)
    }

    pub fn set_checked(&mut self, node: NodeId, checked: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::CHECKED, checked)
    }

    pub fn set_mixed(&mut self, node: NodeId, mixed: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::MIXED, mixed)
    }

    pub fn set_selected(&mut self, node: NodeId, selected: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::SELECTED, selected)
    }

    pub fn set_expanded(&mut self, node: NodeId, expanded: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::EXPANDED, expanded)
    }

    pub fn set_invalid(&mut self, node: NodeId, invalid: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::INVALID, invalid)
    }

    pub fn set_active(&mut self, node: NodeId, active: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::ACTIVE, active)
    }

    pub fn set_highlighted(&mut self, node: NodeId, highlighted: bool) -> bool {
        self.set_interaction_flag(node, InteractionFlags::HIGHLIGHTED, highlighted)
    }

    pub fn set_control_value(&mut self, node: NodeId, value: f32) -> bool {
        if !self.nodes.contains(node) || !value.is_finite() {
            return false;
        }
        if self.interactions.get(node).is_none() {
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let value = value.clamp(0.0, 1.0);
        let interaction_changed = self.interactions.get_mut(node).is_some_and(|interaction| {
            if interaction.value == value {
                false
            } else {
                interaction.value = value;
                interaction.revision = interaction.revision.wrapping_add(1).max(1);
                true
            }
        });
        let semantic_changed = self.semantics.get_mut(node).is_some_and(|semantic| {
            let SemanticValue::Number { current, .. } = &mut semantic.value else {
                return false;
            };
            let value = f64::from(value);
            if *current == value {
                false
            } else {
                *current = value;
                true
            }
        });
        if interaction_changed || semantic_changed {
            self.nodes
                .mark_dirty(node, DirtyFlags::SEMANTICS | DirtyFlags::PAINT);
        }
        interaction_changed || semantic_changed
    }

    /// Returns the nearest explicitly registered control at or above a hit node.
    pub fn nearest_control(&self, mut node: NodeId) -> Option<NodeId> {
        loop {
            if self.interactions.get(node).is_some_and(|interaction| {
                interaction.behavior != ControlBehavior::None && interaction.visible
            }) {
                return Some(node);
            }
            node = self.nodes.core(node)?.parent?;
        }
    }

    pub fn is_descendant_or_self(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut cursor = Some(node);
        while let Some(current) = cursor {
            if current == ancestor {
                return true;
            }
            cursor = self.nodes.core(current).and_then(|core| core.parent);
        }
        false
    }

    pub fn set_hover_within(&mut self, node: NodeId, enabled: bool) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if !enabled {
                return false;
            }
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self.interactions.get_mut(node).unwrap();
        if interaction.hover_within == enabled {
            return false;
        }
        interaction.hover_within = enabled;
        interaction.revision = interaction.revision.wrapping_add(1).max(1);
        true
    }

    pub fn set_control_behavior(&mut self, node: NodeId, behavior: ControlBehavior) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        if interaction.behavior == behavior {
            return false;
        }
        interaction.behavior = behavior;
        interaction.revision = interaction.revision.wrapping_add(1).max(1);
        true
    }

    pub fn set_listener_mask(&mut self, node: NodeId, mask: u16) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        if self.interactions.get(node).is_none() {
            if mask == 0 {
                return false;
            }
            self.interactions
                .insert(node, InteractionSnapshot::default());
        }
        let interaction = self
            .interactions
            .get_mut(node)
            .expect("interaction was inserted above");
        if interaction.listener_mask == mask {
            return false;
        }
        interaction.listener_mask = mask;
        true
    }

    /// Atomically replaces a mounted semantic input and marks only semantic work dirty.
    pub fn set_semantics(
        &mut self,
        node: NodeId,
        semantic: SemanticNode,
    ) -> Result<bool, SemanticError> {
        let validation = if !self.nodes.contains(node) {
            Err(SemanticError::UnknownNode(node))
        } else {
            semantic.validate(node).and_then(|()| {
                if let Some(string) = semantic
                    .referenced_strings()
                    .find(|string| self.string(*string).is_none())
                {
                    return Err(SemanticError::UnknownString(string));
                }
                semantic
                    .relationships
                    .iter()
                    .find(|relationship| !self.nodes.contains(relationship.target))
                    .map_or(Ok(()), |relationship| {
                        Err(SemanticError::UnknownRelationshipTarget(
                            relationship.target,
                        ))
                    })
            })
        };
        if let Err(error) = validation {
            self.diagnostics.semantic_failures += 1;
            return Err(error);
        }
        if self
            .semantics
            .get(node)
            .is_some_and(|current| current == &semantic)
        {
            return Ok(false);
        }
        self.semantics.insert(node, semantic);
        self.nodes.mark_dirty(node, DirtyFlags::SEMANTICS);
        if let Some(core) = self.nodes.core_mut(node) {
            core.semantic_revision += 1;
        }
        self.diagnostics.semantic_updates += 1;
        Ok(true)
    }

    pub fn clear_semantics(&mut self, node: NodeId) -> bool {
        if self.semantics.remove(node).is_none() {
            return false;
        }
        self.nodes.mark_dirty(node, DirtyFlags::SEMANTICS);
        if let Some(core) = self.nodes.core_mut(node) {
            core.semantic_revision += 1;
        }
        self.diagnostics.semantic_updates += 1;
        true
    }

    /// Associates protocol-neutral window-chrome meaning with a normal mounted node.
    ///
    /// Chrome roles do not alter layout or paint. Window hosts consume the role together with
    /// the node's computed bounds after layout, which keeps decoration geometry authored by the
    /// same composition primitives as the rest of the UI.
    #[doc(hidden)]
    pub fn set_window_chrome_role(
        &mut self,
        node: NodeId,
        role: Option<crate::shell::window_chrome::WindowChromeRole>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match role {
            Some(role) => {
                if self.window_chrome_roles.get(node).copied() == Some(role) {
                    false
                } else {
                    self.window_chrome_roles.insert(node, role);
                    true
                }
            }
            None => self.window_chrome_roles.remove(node).is_some(),
        }
    }

    /// Associates hit-test tuning with a semantic window-chrome node.
    #[doc(hidden)]
    pub fn set_window_chrome_hit_spec(
        &mut self,
        node: NodeId,
        spec: Option<crate::shell::window_chrome::WindowChromeHitSpec>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match spec {
            Some(spec) => {
                if self.window_chrome_hit_specs.get(node).copied() == Some(spec) {
                    false
                } else {
                    self.window_chrome_hit_specs.insert(node, spec);
                    true
                }
            }
            None => self.window_chrome_hit_specs.remove(node).is_some(),
        }
    }

    /// Associates a semantic pointer request with a normal mounted node.
    #[doc(hidden)]
    pub fn set_pointer_request(
        &mut self,
        node: NodeId,
        request: Option<crate::assets::PointerRequest>,
    ) -> bool {
        if !self.nodes.contains(node) {
            return false;
        }
        match request {
            Some(request) => {
                if self.pointer_requests.get(node).copied() == Some(request) {
                    false
                } else {
                    self.pointer_requests.insert(node, request);
                    true
                }
            }
            None => self.pointer_requests.remove(node).is_some(),
        }
    }
}

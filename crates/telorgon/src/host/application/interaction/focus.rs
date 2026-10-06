use super::*;

impl InteractionRouter {
    pub(super) fn in_focus_scope(&self, ui: &MountedUi, node: NodeId) -> bool {
        self.focus_scope
            .is_none_or(|scope| ui.is_descendant_or_self(node, scope))
    }

    pub(crate) fn scoped_focus_order(&self, ui: &MountedUi, order: Vec<NodeId>) -> Vec<NodeId> {
        order
            .into_iter()
            .filter(|node| self.in_focus_scope(ui, *node))
            .collect()
    }

    pub(crate) fn sync_focus_requests(&mut self, ui: &mut MountedUi) -> FocusChange {
        let old = self.focused;
        if self.deactivated {
            return FocusChange { old, new: old };
        }
        let nodes = ui.nodes.preorder().to_vec();
        let scope = nodes.iter().rev().copied().find(|node| {
            ui.interactions
                .get(*node)
                .is_some_and(|state| state.focus_scope && state.visible && state.enabled)
        });
        let mut restore = None;
        while let Some((previous_scope, previous_focus)) = self.scope_restore.last().copied() {
            if scope.is_some_and(|scope| ui.is_descendant_or_self(scope, previous_scope)) {
                break;
            }
            self.scope_restore.pop();
            restore = Some(previous_focus);
        }
        let changed_scope = self.focus_scope != scope;
        if let Some(scope) = scope
            && self
                .scope_restore
                .last()
                .is_none_or(|(previous, _)| *previous != scope)
        {
            self.scope_restore.push((scope, self.focused));
        }
        self.focus_scope = scope;
        let eligible = |node: NodeId, ui: &MountedUi| {
            self.in_focus_scope(ui, node)
                && ui.interactions.get(node).is_some_and(|state| {
                    state.enabled
                        && state.visible
                        && state.focusable
                        && state.behavior != ControlBehavior::None
                })
        };
        let mut requested = None;
        for node in &nodes {
            if ui
                .interactions
                .get(*node)
                .is_some_and(|state| state.focus_requested)
            {
                if eligible(*node, ui) {
                    requested = Some(*node);
                }
                ui.interactions.get_mut(*node).unwrap().focus_requested = false;
            }
        }
        let restored = restore.flatten().filter(|node| eligible(*node, ui));
        let current = self.focused.filter(|node| eligible(*node, ui));
        let first = || nodes.iter().copied().find(|node| eligible(*node, ui));
        let target = requested
            .or(restored)
            .or(current)
            .or_else(|| (scope.is_some() && changed_scope).then(first).flatten());
        if requested.is_some() || changed_scope {
            self.set_focus(ui, target, true);
        }
        FocusChange {
            old,
            new: self.focused,
        }
    }

    pub(crate) fn view_activated(&mut self, ui: &mut MountedUi) -> FocusChange {
        self.deactivated = false;
        let target = self.suspended_focus.take();
        self.set_focus(ui, target, false)
    }
}

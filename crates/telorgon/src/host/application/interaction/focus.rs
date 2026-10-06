use super::*;

impl InteractionRouter {
    pub(super) fn in_focus_scope(&self, ui: &MountedUi, node: NodeId) -> bool {
        self.focus_scope
            .is_none_or(|scope| ui.is_descendant_or_self(node, scope))
    }

    pub(crate) fn scoped_focus_order(&self, ui: &MountedUi, order: Vec<NodeId>) -> Vec<NodeId> {
        order
            .into_iter()
            .filter(|node| self.focus_eligible(ui, *node))
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
                .is_some_and(|state| state.focus_scope)
                && self.control_available(ui, *node)
        });
        let mut restore = None;
        let mut index = 0;
        while index < self.scope_restore.len() {
            let (previous_scope, previous_focus) = self.scope_restore[index];
            if ui
                .interactions
                .get(previous_scope)
                .is_some_and(|state| state.focus_scope)
                && self.control_available(ui, previous_scope)
            {
                index += 1;
                continue;
            }
            self.scope_restore.remove(index);
            if let Some(next) = self.scope_restore.get_mut(index) {
                // The next scope was entered from this one. Preserve its outer return
                // link even when the closing scope and its focused descendant are gone.
                next.1 = previous_focus;
            } else {
                restore = Some(previous_focus);
            }
        }
        let changed_scope = self.focus_scope != scope;
        if let Some(scope) = scope
            && !self
                .scope_restore
                .iter()
                .any(|(previous, _)| *previous == scope)
        {
            let return_owner = restore.unwrap_or_else(|| {
                self.focused
                    .or(self.suspended_focus.map(|(owner, _)| owner))
            });
            self.scope_restore.push((scope, return_owner));
        }
        self.focus_scope = scope;
        let mut requested = None;
        for node in &nodes {
            if ui
                .interactions
                .get(*node)
                .is_some_and(|state| state.focus_requested)
            {
                if self.focus_eligible(ui, *node) {
                    requested = Some(*node);
                }
                ui.interactions.get_mut(*node).unwrap().focus_requested = false;
            }
        }
        let restored = restore
            .flatten()
            .filter(|node| self.focus_eligible(ui, *node));
        let current = self.focused.filter(|node| self.focus_eligible(ui, *node));
        let first = || {
            nodes
                .iter()
                .copied()
                .find(|node| self.focus_eligible(ui, *node))
        };
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
}

#[cfg(test)]
mod tests;

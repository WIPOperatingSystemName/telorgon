use super::*;

impl CompositionDriver {
    pub(super) fn root_node(&self, mounted: &MountedElement) -> Option<UiNodeId> {
        match &mounted.kind {
            MountedKind::Container { node, .. }
            | MountedKind::Text { node, .. }
            | MountedKind::Image { node, .. }
            | MountedKind::Button { node, .. }
            | MountedKind::Checkbox { node, .. }
            | MountedKind::Switch { node, .. }
            | MountedKind::Slider { node, .. } => Some(*node),
            MountedKind::Component { id, .. } => self
                .arena
                .get(*id)
                .and_then(|slot| slot.child.as_deref())
                .and_then(|child| self.root_node(child)),
        }
    }

    pub(super) fn update_component_candidate(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        id: ComponentInstanceId,
        candidate: Box<dyn ErasedComponent>,
    ) -> Result<bool, ViewError> {
        let changed = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .ok_or(ViewError::StaleParent)?
            .update_from(candidate)?;
        if !changed {
            self.diagnostics.components_reused += 1;
            return Ok(false);
        }
        let requested = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .is_some_and(|component| component.inputs_changed_erased(id));
        let _ = requested;
        self.reconcile_component(ui, id)?;
        Ok(true)
    }

    pub(super) fn reconcile_component(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        id: ComponentInstanceId,
    ) -> Result<(), ViewError> {
        let rendered = self.render_component(id)?;
        let old = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.child.take())
            .ok_or(ViewError::StaleParent)?;
        let parent = self
            .root_node(&old)
            .and_then(|node| ui.nodes.core(node).and_then(|core| core.parent))
            .ok_or(ViewError::StaleParent)?;
        match self.reconcile_element(ui, parent, *old, rendered.element, id) {
            Ok(child) => {
                self.arena.get_mut(id).ok_or(ViewError::StaleParent)?.child = Some(Box::new(child));
                self.commit_signal_dependencies(id, rendered.signals)?;
                Ok(())
            }
            Err((old, error)) => {
                if let Some(slot) = self.arena.get_mut(id) {
                    slot.child = Some(Box::new(old));
                }
                Err(error)
            }
        }
    }

    pub(super) fn reconcile_element(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        parent: UiNodeId,
        mut old: MountedElement,
        candidate: Element,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, (MountedElement, ViewError)> {
        let candidate_type = candidate.kind().identity();
        let same_identity =
            old.key.as_ref() == candidate.key_ref() && old.element_type() == candidate_type;
        if !same_identity {
            let Some(old_root) = self.root_node(&old) else {
                return Err((old, ViewError::StaleParent));
            };
            let Some(mut writer) = MountWriter::under(ui, parent) else {
                return Err((old, ViewError::StaleParent));
            };
            let mounted = match self.mount_element(&mut writer, candidate, owner) {
                Ok(mounted) => mounted,
                Err(error) => return Err((old, error)),
            };
            let new_root = self
                .root_node(&mounted)
                .expect("mounted element has a root");
            ui.nodes.reparent_before(new_root, parent, Some(old_root));
            self.remove_mounted(ui, old);
            return Ok(mounted);
        }

        let (key, kind, window_chrome_role, window_chrome_hit_spec, pointer_request) =
            candidate.into_parts();
        let result = match (&mut old.kind, kind) {
            (
                MountedKind::Container {
                    node,
                    style,
                    layout,
                    children,
                },
                ElementKind::Container(candidate),
            ) if (ui.kinds.get(*node) == Some(&crate::ui::NodeKind::Scroll)) == candidate.scrollable => {
                ui.set_box_style(*node, candidate.style);
                let mut next_layout = candidate.layout;
                if candidate.scrollable {
                    next_layout.scroll_offset = ui.layouts.get(*node).map_or(crate::PointF::default(), |l| l.scroll_offset);
                }
                ui.set_layout_style(*node, next_layout);
                ui.set_hover_within(*node, candidate.hover_within);
                if let Some(style) = candidate.inline_style.as_ref() {
                    ui.set_style_id(*node, style.id);
                    if !ui
                        .style_bindings()
                        .iter()
                        .any(|binding| binding.state_root == *node)
                    {
                        ui.register_style_binding(
                            StyleBinding::new(*node, ThemeScopeId::new(0, 1), style.id)
                                .slot(StyleSlotId::named("root"), *node)
                                .local_style(style.clone()),
                        );
                    }
                }
                ui.set_local_component_style(*node, candidate.inline_style.clone());
                *style = candidate.style;
                *layout = candidate.layout;
                let previous = std::mem::take(children);
                *children =
                    match self.reconcile_children(ui, *node, previous, candidate.children, owner) {
                        Ok(children) => children,
                        Err((previous, error)) => {
                            *children = previous;
                            return Err((old, error));
                        }
                    };
                Ok(())
            }
            (MountedKind::Text { node, props }, ElementKind::Text(candidate)) => {
                ui.set_dynamic_text(*node, &candidate.content);
                let style = candidate.style.resolve_with(|family| ui.intern(family));
                ui.set_text_style(*node, style);
                ui.set_box_style(*node, candidate.box_style);
                ui.set_layout_style(*node, candidate.layout);
                *props = candidate;
                Ok(())
            }
            (MountedKind::Image { node, props }, ElementKind::Image(candidate)) => {
                if props.image != candidate.image
                    || props.content_version != candidate.content_version
                    || props.tint != candidate.tint
                {
                    ui.set_image_visual_tinted(
                        *node,
                        candidate.image,
                        candidate.content_version,
                        candidate.tint,
                    );
                }
                ui.set_box_style(*node, candidate.style);
                ui.set_layout_style(*node, candidate.layout);
                match &candidate.accessible_label {
                    Some(label) => {
                        let name = ui.intern(label);
                        let _ = ui.set_semantics(
                            *node,
                            SemanticNode {
                                role: SemanticRole::Image,
                                name: SemanticName::Text(name),
                                ..SemanticNode::default()
                            },
                        );
                    }
                    None => {
                        ui.semantics.remove(*node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Button {
                    node,
                    children,
                    props,
                },
                ElementKind::Button(mut candidate),
            ) => {
                // The mounted style includes resolved theme/inline properties. A parent
                // update must not reset those to identical authored defaults: unchanged
                // bindings will not resolve again until an interaction state changes.
                if props.style != candidate.style || (props.hover_properties & !candidate.hover_properties != 0) {
                    ui.set_box_style(*node, candidate.style);
                    ui.reset_local_style_motion(*node);
                }
                let previous = std::mem::take(children);
                *children = match self.reconcile_children(
                    ui, *node, previous, std::mem::take(&mut candidate.children), owner,
                ) {
                    Ok(children) => children,
                    Err((previous, error)) => {
                        *children = previous;
                        return Err((old, error));
                    }
                };
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_busy(*node, candidate.busy);
                ui.set_style_id(*node, candidate.style_id);
                ui.set_style_override(*node, StyleSlotId::named("root"), candidate.style_override);
                ui.set_local_component_style(*node, candidate.inline_style.clone());
                ui.set_local_style_overlay(*node, candidate.hover_overlay);
                let name = candidate.accessible_label.as_ref()
                    .map(|label| SemanticName::Text(ui.intern(label)))
                    .unwrap_or(SemanticName::Contents);
                let _ = ui.set_semantics(*node, button_semantics(name, &candidate));
                match &candidate.on_press {
                    Some(handler) => {
                        self.handlers
                            .insert(*node, HandlerRoute::Activate(handler.bind(owner)));
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Checkbox {
                    node,
                    indicator,
                    check_first,
                    check_second,
                    mixed,
                    label,
                    props,
                },
                ElementKind::Toggle(candidate),
            ) => {
                let styles = checkbox_styles(candidate.value, candidate.enabled);
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*indicator, styles.indicator);
                ui.set_box_style(*check_first, styles.check_first);
                ui.set_box_style(*check_second, styles.check_second);
                ui.set_box_style(*mixed, styles.mixed);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_checked(*node, candidate.value != SemanticCheckState::Unchecked);
                ui.set_mixed(*node, candidate.value == SemanticCheckState::Mixed);
                let name = ui.intern(&candidate.label);
                let _ = ui.set_semantics(*node, toggle_semantics(name, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers.insert(
                            *node,
                            HandlerRoute::Toggle {
                                handler: handler.bind(owner),
                                value: candidate.value,
                            },
                        );
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Switch {
                    node,
                    track,
                    thumb,
                    label,
                    props,
                },
                ElementKind::Toggle(candidate),
            ) => {
                let mut styles = switch_styles(
                    candidate.value == SemanticCheckState::Checked,
                    candidate.enabled,
                );
                if let Some(width) = candidate.width { styles.container.width = crate::ui::SizeRule::Logical(width); }
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*track, styles.track);
                ui.set_box_style(*thumb, styles.thumb);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_checked(*node, candidate.value == SemanticCheckState::Checked);
                ui.set_mixed(*node, false);
                let name = ui.intern(&candidate.label);
                let _ = ui.set_semantics(*node, toggle_semantics(name, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers.insert(
                            *node,
                            HandlerRoute::Toggle {
                                handler: handler.bind(owner),
                                value: candidate.value,
                            },
                        );
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (
                MountedKind::Slider {
                    before_thumb,
                    after_thumb,
                    node,
                    track,
                    fill,
                    thumb,
                    label,
                    props,
                },
                ElementKind::Slider(candidate),
            ) => {
                let styles = slider_styles(candidate.value, candidate.enabled, candidate.width.into());
                ui.set_box_style(*node, styles.container);
                ui.set_box_style(*track, styles.track);
                ui.set_box_style(*fill, styles.fill);
                ui.set_box_style(*thumb, styles.thumb);
                ui.set_box_style(*before_thumb, styles.before_thumb);
                ui.set_box_style(*after_thumb, styles.after_thumb);
                ui.set_dynamic_text(*label, &candidate.label);
                ui.set_text_style(*label, control_label_style(candidate.enabled));
                ui.set_disabled(*node, !candidate.enabled);
                ui.set_control_value(*node, candidate.value);
                let name = ui.intern(candidate.accessible_label.as_deref().unwrap_or(&candidate.label));
                let value_text = ui.intern(format!("{:.0}%", candidate.value * 100.0));
                let _ = ui.set_semantics(*node, slider_semantics(name, value_text, &candidate));
                match &candidate.on_change {
                    Some(handler) => {
                        self.handlers
                            .insert(*node, HandlerRoute::Value(handler.bind(owner)));
                    }
                    None => {
                        self.handlers.remove(node);
                    }
                }
                *props = candidate;
                Ok(())
            }
            (MountedKind::Component { id, .. }, ElementKind::Component(candidate)) => self
                .update_component_candidate(ui, *id, candidate)
                .map(|_| ()),
            _ => unreachable!("equal element identities have equal payload variants"),
        };
        if let Err(error) = result {
            return Err((old, error));
        }
        let root = self
            .root_node(&old)
            .expect("every reconciled composition element has a root node");
        ui.set_window_chrome_role(root, window_chrome_role);
        ui.set_window_chrome_hit_spec(root, window_chrome_hit_spec);
        ui.set_pointer_request(root, pointer_request);
        old.key = key;
        self.diagnostics.elements_reused += 1;
        Ok(old)
    }

    pub(super) fn reconcile_children(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        parent: UiNodeId,
        old: Vec<MountedElement>,
        candidates: Vec<Element>,
        owner: ComponentInstanceId,
    ) -> Result<Vec<MountedElement>, (Vec<MountedElement>, ViewError)> {
        let mut old: Vec<Option<MountedElement>> = old.into_iter().map(Some).collect();
        let mut next = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let candidate_type = candidate.kind().identity();
            let match_index = if let Some(key) = candidate.key_ref() {
                old.iter().position(|entry| {
                    entry.as_ref().is_some_and(|mounted| {
                        mounted.key.as_ref() == Some(key)
                            && mounted.element_type() == candidate_type
                    })
                })
            } else {
                old.get(index).and_then(|entry| {
                    entry.as_ref().and_then(|mounted| {
                        (mounted.key.is_none() && mounted.element_type() == candidate_type)
                            .then_some(index)
                    })
                })
            };
            let mounted = if let Some(match_index) = match_index {
                let mounted = old[match_index].take().expect("matched child exists");
                match self.reconcile_element(ui, parent, mounted, candidate, owner) {
                    Ok(mounted) => mounted,
                    Err((mounted, error)) => {
                        old[match_index] = Some(mounted);
                        let mut restored = next;
                        restored.extend(old.into_iter().flatten());
                        return Err((restored, error));
                    }
                }
            } else {
                let Some(mut writer) = MountWriter::under(ui, parent) else {
                    let mut restored = next;
                    restored.extend(old.into_iter().flatten());
                    return Err((restored, ViewError::StaleParent));
                };
                match self.mount_element(&mut writer, candidate, owner) {
                    Ok(mounted) => mounted,
                    Err(error) => {
                        let mut restored = next;
                        restored.extend(old.into_iter().flatten());
                        return Err((restored, error));
                    }
                }
            };
            next.push(mounted);
        }
        for removed in old.into_iter().flatten() {
            self.remove_mounted(ui, removed);
        }
        let mut before = None;
        for child in next.iter().rev() {
            let node = self.root_node(child).expect("mounted child has a root");
            ui.nodes.reparent_before(node, parent, before);
            before = Some(node);
        }
        Ok(next)
    }

    pub(super) fn teardown_metadata(&mut self, mut mounted: MountedElement) {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        match &mut mounted.kind {
            MountedKind::Container { children, .. } => {
                for child in std::mem::take(children) {
                    self.teardown_metadata(child);
                }
            }
            MountedKind::Text { .. } | MountedKind::Image { .. } => {}
            MountedKind::Button { node, children, .. } => {
                self.handlers.remove(node);
                for child in std::mem::take(children) {
                    self.teardown_metadata(child);
                }
            }
            MountedKind::Checkbox { node, .. }
            | MountedKind::Switch { node, .. }
            | MountedKind::Slider { node, .. } => {
                self.handlers.remove(node);
            }
            MountedKind::Component { id, .. } => {
                if let Some(child) = self.arena.get_mut(*id).and_then(|slot| slot.child.take()) {
                    self.teardown_metadata(*child);
                }
                if let Some(component) = self
                    .arena
                    .get_mut(*id)
                    .and_then(|slot| slot.component.as_deref_mut())
                {
                    component.unmounted_erased(*id);
                }
                self.arena.remove(*id);
                self.diagnostics.components_unmounted += 1;
            }
        }
    }

    pub(super) fn remove_mounted(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        mounted: MountedElement,
    ) {
        if let Some(node) = self.root_node(&mounted) {
            self.teardown_metadata(mounted);
            ui.remove(node);
            self.diagnostics.elements_removed += 1;
        }
    }
}

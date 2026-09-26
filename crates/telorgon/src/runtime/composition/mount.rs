use super::*;

impl CompositionDriver {
    pub(super) fn mount_component(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        component: Box<dyn ErasedComponent>,
    ) -> Result<MountedElement, ViewError> {
        let _scope = crate::authoring::compose::context::ProviderGuard::enter_with_images(
            self.shell_services.clone(),
            self.image_bindings.clone(),
        );
        let type_id = component.component_type_id();
        let id = self.arena.insert(component);
        let rendered = match self.render_component(id) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.arena.remove(id);
                return Err(error);
            }
        };
        let child = match self.mount_element(writer, rendered.element, id) {
            Ok(child) => child,
            Err(error) => {
                self.arena.remove(id);
                return Err(error);
            }
        };
        self.arena.get_mut(id).expect("new component is live").child = Some(Box::new(child));
        self.commit_signal_dependencies(id, rendered.signals)?;
        let requested = self
            .arena
            .get_mut(id)
            .and_then(|slot| slot.component.as_deref_mut())
            .is_some_and(|component| component.mounted_erased(id));
        if requested {
            self.signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned")
                .push(id);
        }
        self.diagnostics.components_mounted += 1;
        Ok(MountedElement {
            key: None,
            kind: MountedKind::Component { id, type_id },
        })
    }

    pub(super) fn mount_element(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        element: Element,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        let (key, kind, window_chrome_role, window_chrome_hit_spec, pointer_request) =
            element.into_parts();
        let mut mounted = match kind {
            ElementKind::Container(ContainerElement {
                inline_style,
                hover_within,
                scrollable,
                style,
                layout,
                children,
            }) => {
                let mut mounted_children = Vec::with_capacity(children.len());
                let content = |writer: &mut MountWriter<'_, ()>| {
                    for child in children {
                        mounted_children.push(self.mount_element(writer, child, owner));
                    }
                };
                let node = if scrollable {
                    writer.scroll(style, layout, content).node
                } else {
                    writer.container(style, layout, content)
                };
                let children = mounted_children
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                writer.hover_within(node, hover_within);
                if let Some(style) = inline_style {
                    writer.style_id(node, style.id);
                    writer.style_binding(
                        StyleBinding::new(node, ThemeScopeId::new(0, 1), style.id)
                            .slot(StyleSlotId::named("root"), node)
                            .local_style(style),
                    );
                }
                MountedElement {
                    key: None,
                    kind: MountedKind::Container {
                        node,
                        style,
                        layout,
                        children,
                    },
                }
            }
            ElementKind::Text(props) => {
                let style = props.style.resolve_with(|family| writer.intern(family));
                let text = writer.dynamic_text(
                    props.content.clone(),
                    style,
                    props.box_style,
                    props.layout,
                );
                MountedElement {
                    key: None,
                    kind: MountedKind::Text {
                        node: text.node,
                        props,
                    },
                }
            }
            ElementKind::Image(props) => {
                let image = writer.dynamic_image_tinted(
                    props.image,
                    props.content_version,
                    props.tint,
                    props.style,
                    props.layout,
                );
                if let Some(label) = &props.accessible_label {
                    let name = writer.intern(label);
                    writer
                        .semantic_node(
                            image.node,
                            SemanticNode {
                                role: SemanticRole::Image,
                                name: SemanticName::Text(name),
                                ..SemanticNode::default()
                            },
                        )
                        .map_err(|_| ViewError::MissingButtonLabel)?;
                }
                MountedElement {
                    key: None,
                    kind: MountedKind::Image {
                        node: image.node,
                        props,
                    },
                }
            }
            ElementKind::Button(mut props) => {
                let mut mounted_children = Vec::new();
                let control = writer.button_node(props.style, |writer| {
                    for child in std::mem::take(&mut props.children) {
                        mounted_children.push(self.mount_element(writer, child, owner));
                    }
                });
                let children = mounted_children.into_iter().collect::<Result<Vec<_>, _>>()?;
                writer.hover_within(control.node, true);
                writer.style_id(control.node, props.style_id);
                let mut binding =
                    StyleBinding::new(control.node, ThemeScopeId::new(0, 1), props.style_id)
                        .slot(StyleSlotId::named("root"), control.node);
                if let Some(slot) = props.content_style_slot
                    && let Some(node) = children.first().and_then(|child| self.root_node(child))
                {
                    binding = binding.slot(slot, node);
                }
                if let Some(style) = props.inline_style.clone() {
                    binding = binding.local_style(style);
                }
                binding.local_style_overlay = props.effect_overlay;
                writer.style_binding(binding);
                if props.style_override != StylePropertyPatch::default() {
                    writer.style_override(
                        control.node,
                        StyleSlotId::named("root"),
                        props.style_override,
                    );
                }
                writer.disabled(control.node, !props.enabled);
                writer.busy(control.node, props.busy);
                let name = props.accessible_label.as_ref()
                    .map(|label| SemanticName::Text(writer.intern(label)))
                    .unwrap_or(SemanticName::Contents);
                writer
                    .semantic_node(control.node, button_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_press {
                    self.handlers
                        .insert(control.node, HandlerRoute::Activate(handler.bind(owner)));
                }
                MountedElement {
                    key: None,
                    kind: MountedKind::Button {
                        node: control.node,
                        children,
                        props,
                    },
                }
            }
            ElementKind::Toggle(props) => self.mount_toggle(writer, props, owner)?,
            ElementKind::Slider(props) => self.mount_slider(writer, props, owner)?,
            ElementKind::Component(component) => self.mount_component(writer, component)?,
        };
        mounted.key = key;
        let root = self
            .root_node(&mounted)
            .expect("every mounted composition element has a root node");
        writer.window_chrome_role(root, window_chrome_role);
        writer.window_chrome_hit_spec(root, window_chrome_hit_spec);
        writer.pointer_request(root, pointer_request);
        self.diagnostics.elements_mounted += 1;
        Ok(mounted)
    }

    pub(super) fn mount_toggle(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        props: ToggleElement,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        match props.kind {
            ToggleKind::Checkbox => {
                let styles = checkbox_styles(props.value, props.enabled);
                let mut indicator = None;
                let mut check_first = None;
                let mut check_second = None;
                let mut mixed = None;
                let mut label = None;
                let control = writer.toggle_node(styles.container, |writer| {
                    writer.container(
                        BoxStyle::default(),
                        LayoutStyle {
                            flow: Flow::Horizontal,
                            gap: 8.0,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            indicator = Some(
                                writer
                                    .container_handle(
                                        styles.indicator,
                                        LayoutStyle {
                                            flow: Flow::Overlay,
                                            ..LayoutStyle::default()
                                        },
                                        |writer| {
                                            check_first = Some(
                                                writer
                                                    .container_handle(
                                                        styles.check_first,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                            check_second = Some(
                                                writer
                                                    .container_handle(
                                                        styles.check_second,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                            mixed = Some(
                                                writer
                                                    .container_handle(
                                                        styles.mixed,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                        },
                                    )
                                    .node,
                            );
                            label = Some(
                                writer
                                    .dynamic_text(
                                        props.label.clone(),
                                        control_label_style(props.enabled),
                                        BoxStyle::default(),
                                        LayoutStyle::default(),
                                    )
                                    .node,
                            );
                        },
                    );
                });
                writer.style_id(
                    control.node,
                    ComponentStyleId::named(ThemeDomainId::APPLICATION, "checkbox", "default"),
                );
                writer.style_binding(
                    StyleBinding::new(
                        control.node,
                        ThemeScopeId::new(0, 1),
                        ComponentStyleId::named(ThemeDomainId::APPLICATION, "checkbox", "default"),
                    )
                    .slot(StyleSlotId::named("root"), control.node)
                    .slot(
                        StyleSlotId::named("indicator"),
                        indicator.expect("checkbox mounts its indicator"),
                    )
                    .slot(
                        StyleSlotId::named("check-start"),
                        check_first.expect("checkbox mounts its first check segment"),
                    )
                    .slot(
                        StyleSlotId::named("check-end"),
                        check_second.expect("checkbox mounts its second check segment"),
                    )
                    .slot(
                        StyleSlotId::named("mixed"),
                        mixed.expect("checkbox mounts its mixed segment"),
                    )
                    .slot(
                        StyleSlotId::named("label"),
                        label.expect("checkbox mounts its label"),
                    ),
                );
                writer.disabled(control.node, !props.enabled);
                writer.checked(control.node, props.value != SemanticCheckState::Unchecked);
                let name = writer.intern(&props.label);
                writer
                    .semantic_node(control.node, toggle_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_change {
                    self.handlers.insert(
                        control.node,
                        HandlerRoute::Toggle {
                            handler: handler.bind(owner),
                            value: props.value,
                        },
                    );
                }
                Ok(MountedElement {
                    key: None,
                    kind: MountedKind::Checkbox {
                        node: control.node,
                        indicator: indicator.expect("checkbox indicator was bound"),
                        check_first: check_first.expect("checkbox first check was bound"),
                        check_second: check_second.expect("checkbox second check was bound"),
                        mixed: mixed.expect("checkbox mixed mark was bound"),
                        label: label.expect("checkbox label was bound"),
                        props,
                    },
                })
            }
            ToggleKind::Switch => {
                let mut styles =
                    switch_styles(props.value == SemanticCheckState::Checked, props.enabled);
                if let Some(width) = props.width { styles.container.width = crate::ui::SizeRule::Logical(width); }
                let mut track = None;
                let mut thumb = None;
                let mut label = None;
                let control = writer.toggle_node(styles.container, |writer| {
                    writer.container(
                        BoxStyle { width: crate::ui::SizeRule::Fill(1.0), ..Default::default() },
                        LayoutStyle {
                            flow: Flow::Horizontal,
                            gap: 8.0,
                            ..LayoutStyle::default()
                        },
                        |writer| {
                            track = Some(
                                writer
                                    .container_handle(
                                        styles.track,
                                        LayoutStyle::default(),
                                        |writer| {
                                            thumb = Some(
                                                writer
                                                    .container_handle(
                                                        styles.thumb,
                                                        LayoutStyle::default(),
                                                        |_| {},
                                                    )
                                                    .node,
                                            );
                                        },
                                    )
                                    .node,
                            );
                            label = Some(
                                writer
                                    .dynamic_text(
                                        props.label.clone(),
                                        control_label_style(props.enabled),
                                        BoxStyle {width: crate::ui::SizeRule::Fill(1.0), ..Default::default()},
                                        LayoutStyle::default(),
                                    )
                                    .node,
                            );
                        },
                    );
                });
                writer.style_id(
                    control.node,
                    ComponentStyleId::named(ThemeDomainId::APPLICATION, "switch", "default"),
                );
                writer.style_binding(
                    StyleBinding::new(
                        control.node,
                        ThemeScopeId::new(0, 1),
                        ComponentStyleId::named(ThemeDomainId::APPLICATION, "switch", "default"),
                    )
                    .slot(StyleSlotId::named("root"), control.node)
                    .slot(
                        StyleSlotId::named("track"),
                        track.expect("switch mounts its track"),
                    )
                    .slot(
                        StyleSlotId::named("thumb"),
                        thumb.expect("switch mounts its thumb"),
                    )
                    .slot(
                        StyleSlotId::named("label"),
                        label.expect("switch mounts its label"),
                    ),
                );
                writer.disabled(control.node, !props.enabled);
                writer.checked(control.node, props.value == SemanticCheckState::Checked);
                let name = writer.intern(&props.label);
                writer
                    .semantic_node(control.node, toggle_semantics(name, &props))
                    .map_err(|_| ViewError::MissingButtonLabel)?;
                if let Some(handler) = &props.on_change {
                    self.handlers.insert(
                        control.node,
                        HandlerRoute::Toggle {
                            handler: handler.bind(owner),
                            value: props.value,
                        },
                    );
                }
                Ok(MountedElement {
                    key: None,
                    kind: MountedKind::Switch {
                        node: control.node,
                        track: track.expect("switch track was bound"),
                        thumb: thumb.expect("switch thumb was bound"),
                        label: label.expect("switch label was bound"),
                        props,
                    },
                })
            }
        }
    }

    pub(super) fn mount_slider(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        props: SliderElement,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        let styles = slider_styles(props.value, props.enabled, props.width.into());
        let mut track = None;
        let mut fill = None;
        let mut thumb = None;
        let mut before_thumb = None;
        let mut after_thumb = None;
        let mut label = None;
        let control = writer.slider_node(styles.container, |writer| {
            writer.container(
                BoxStyle {
                    width: SizeRule::Fill(1.0),
                    height: SizeRule::Logical(22.0),
                    ..BoxStyle::default()
                },
                LayoutStyle {
                    flow: Flow::Horizontal,
                    cross_axis_alignment: crate::ui::CrossAxisAlignment::Center,
                    gap: if props.label.is_empty() { 0.0 } else { 8.0 },
                    ..LayoutStyle::default()
                },
                |writer| {
                    label = Some(
                        writer
                            .dynamic_text(
                                props.label.clone(),
                                control_label_style(props.enabled),
                                BoxStyle::default(),
                                LayoutStyle::default(),
                            )
                            .node,
                    );
                    track = Some(
                        writer
                            .container_handle(
                                styles.track,
                                LayoutStyle {
                                    flow: Flow::Overlay,
                                    ..LayoutStyle::default()
                                },
                                |writer| {
                                    fill = Some(
                                        writer
                                            .container_handle(
                                                styles.fill,
                                                LayoutStyle::default(),
                                                |_| {},
                                            )
                                            .node,
                                    );
                                    // Weighted spacers position the thumb within the remaining
                                    // track space without reading or computing the track width.
                                    writer.container(
                                        BoxStyle {
                                            width: SizeRule::Fill(1.0),
                                            height: SizeRule::Logical(18.0),
                                            max_size: SizeRule2D { width: SizeRule::Fill(1.0), height: SizeRule::Logical(18.0) },
                                            transform: Transform2D { translation: PointF { x: 0.0, y: -6.0 }, ..Transform2D::default() },
                                            ..BoxStyle::default()
                                        },
                                        LayoutStyle { flow: Flow::Horizontal, ..LayoutStyle::default() },
                                        |writer| {
                                            before_thumb = Some(writer.container_handle(styles.before_thumb, LayoutStyle::default(), |_| {}).node);
                                            thumb = Some(writer.container_handle(styles.thumb, LayoutStyle::default(), |_| {}).node);
                                            after_thumb = Some(writer.container_handle(styles.after_thumb, LayoutStyle::default(), |_| {}).node);
                                        },
                                    );
                                },
                            )
                            .node,
                    );
                },
            );
        });
        let track = track.expect("slider mounts its track");
        writer.style_id(
            control.node,
            ComponentStyleId::named(ThemeDomainId::APPLICATION, "slider", "default"),
        );
        writer.style_binding(
            StyleBinding::new(
                control.node,
                ThemeScopeId::new(0, 1),
                ComponentStyleId::named(ThemeDomainId::APPLICATION, "slider", "default"),
            )
            .slot(StyleSlotId::named("root"), control.node)
            .slot(StyleSlotId::named("track"), track)
            .slot(
                StyleSlotId::named("fill"),
                fill.expect("slider mounts its fill"),
            )
            .slot(
                StyleSlotId::named("thumb"),
                thumb.expect("slider mounts its thumb"),
            )
            .slot(
                StyleSlotId::named("label"),
                label.expect("slider mounts its label"),
            ),
        );
        writer.disabled(control.node, !props.enabled);
        writer.control_value(control.node, props.value);
        writer.value_track(
            control.node,
            track,
            ValueAxis::Horizontal { inverted: false },
        );
        let name = writer.intern(props.accessible_label.as_deref().unwrap_or(&props.label));
        let value_text = writer.intern(format!("{:.0}%", props.value * 100.0));
        writer
            .semantic_node(control.node, slider_semantics(name, value_text, &props))
            .map_err(|_| ViewError::MissingButtonLabel)?;
        if let Some(handler) = &props.on_change {
            self.handlers
                .insert(control.node, HandlerRoute::Value(handler.bind(owner)));
        }
        Ok(MountedElement {
            key: None,
            kind: MountedKind::Slider {
                before_thumb: before_thumb.expect("slider leading space mounted"),
                after_thumb: after_thumb.expect("slider trailing space mounted"),
                node: control.node,
                track,
                fill: fill.expect("slider fill was bound"),
                thumb: thumb.expect("slider thumb was bound"),
                label: label.expect("slider label was bound"),
                props,
            },
        })
    }
}

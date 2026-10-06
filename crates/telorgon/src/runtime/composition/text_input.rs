use super::*;

fn semantics(
    name: SemanticName,
    value: crate::ui::StringId,
    props: &TextInputElement,
) -> SemanticNode {
    SemanticNode {
        role: SemanticRole::TextInput,
        name,
        value: SemanticValue::Text(value),
        state: SemanticState {
            disabled: !props.enabled,
            read_only: props.read_only,
            focusable: props.enabled,
            ..SemanticState::default()
        },
        actions: if props.enabled {
            SemanticActions::FOCUS
        } else {
            SemanticActions::NONE
        },
        ..SemanticNode::default()
    }
}

impl CompositionDriver {
    pub(super) fn mount_text_input(
        &mut self,
        writer: &mut MountWriter<'_, ()>,
        mut props: TextInputElement,
        owner: ComponentInstanceId,
    ) -> Result<MountedElement, ViewError> {
        let mut mounted = Vec::new();
        let control = writer.text_input_node(
            props.content.style,
            props.content.layout,
            props.enabled,
            |writer| {
                for child in std::mem::take(&mut props.content.children) {
                    mounted.push(self.mount_element(writer, child, owner));
                }
            },
        );
        let children = mounted.into_iter().collect::<Result<Vec<_>, _>>()?;
        writer.text_cursor_rect(control.node, props.ime_cursor_rect);
        let name =
            SemanticName::Text(writer.intern(props.accessible_label.as_deref().unwrap_or("")));
        let value = writer.intern(if props.secure { "" } else { &props.value });
        writer
            .semantic_node(control.node, semantics(name, value, &props))
            .map_err(|_| ViewError::MissingButtonLabel)?;
        if props.autofocus {
            writer.request_focus(control.node);
        }
        if let Some(handler) = &props.on_input {
            self.input_handlers
                .insert(control.node, handler.bind(owner));
        }
        Ok(MountedElement {
            key: None,
            kind: MountedKind::TextInput {
                node: control.node,
                children,
                props,
            },
        })
    }

    pub(super) fn reconcile_text_input(
        &mut self,
        ui: &mut crate::ui::MountedUi,
        node: UiNodeId,
        children: &mut Vec<MountedElement>,
        props: &mut TextInputElement,
        mut candidate: TextInputElement,
        owner: ComponentInstanceId,
    ) -> Result<(), ViewError> {
        if props.content.style != candidate.content.style {
            ui.set_box_style(node, candidate.content.style);
        }
        ui.set_layout_style(node, candidate.content.layout);
        ui.set_disabled(node, !candidate.enabled);
        ui.set_text_cursor_rect(node, candidate.ime_cursor_rect);
        let previous = std::mem::take(children);
        *children = match self.reconcile_children(
            ui,
            node,
            previous,
            std::mem::take(&mut candidate.content.children),
            owner,
        ) {
            Ok(children) => children,
            Err((previous, error)) => {
                *children = previous;
                return Err(error);
            }
        };
        let name =
            SemanticName::Text(ui.intern(candidate.accessible_label.as_deref().unwrap_or("")));
        let value = ui.intern(if candidate.secure {
            ""
        } else {
            &candidate.value
        });
        let _ = ui.set_semantics(node, semantics(name, value, &candidate));
        if candidate.autofocus && !props.autofocus {
            ui.request_focus(node);
        }
        match &candidate.on_input {
            Some(handler) => {
                self.input_handlers.insert(node, handler.bind(owner));
            }
            None => {
                self.input_handlers.remove(&node);
            }
        }
        *props = candidate;
        Ok(())
    }
}

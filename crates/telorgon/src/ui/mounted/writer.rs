use super::*;

#[derive(Debug)]
pub struct MountWriter<'a, A> {
    ui: &'a mut MountedUi,
    parents: Vec<NodeId>,
    action_routes: Vec<(NodeId, A)>,
    owns_view_root: bool,
    mounted_root: bool,
}
impl<'a, A> MountWriter<'a, A> {
    pub fn new(ui: &'a mut MountedUi) -> Self {
        Self {
            ui,
            parents: Vec::with_capacity(16),
            action_routes: Vec::new(),
            owns_view_root: true,
            mounted_root: false,
        }
    }
    /// Creates the narrow mount writer used by the component runtime for one child subtree.
    #[doc(hidden)]
    pub fn under(ui: &'a mut MountedUi, parent: NodeId) -> Option<Self> {
        if !ui.nodes.contains(parent) {
            return None;
        }
        Some(Self {
            ui,
            parents: vec![parent],
            action_routes: Vec::new(),
            owns_view_root: false,
            mounted_root: false,
        })
    }
    /// Drains typed actions staged while mounting so the component runtime can own their routes.
    pub fn drain_action_routes(&mut self) -> impl Iterator<Item = (NodeId, A)> + '_ {
        self.action_routes.drain(..)
    }
    pub fn intern(&mut self, text: impl AsRef<str>) -> StringId {
        self.ui.intern(text)
    }
    pub fn root(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> UiRoot {
        assert!(
            !self.mounted_root,
            "a mount writer creates exactly one root"
        );
        if self.owns_view_root {
            assert!(self.ui.root.is_none(), "a mounted UI has exactly one root");
        }
        let node = self.mount(NodeKind::Box, style, layout, InteractionSnapshot::default());
        self.mounted_root = true;
        if self.owns_view_root {
            self.ui.root = Some(UiRoot(node));
        }
        self.parents.push(node);
        content(self);
        self.parents.pop();
        UiRoot(node)
    }
    pub fn container(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> NodeId {
        let node = self.mount(NodeKind::Box, style, layout, InteractionSnapshot::default());
        self.parents.push(node);
        content(self);
        self.parents.pop();
        node
    }
    /// Creates a noninteractive container and exposes its patchable visual properties.
    #[doc(hidden)]
    pub fn container_handle(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        ControlHandle::new(self.container(style, layout, content))
    }
    /// Mounts an overlay/studio layer whose visibility can be patched without rebuilding it.
    pub fn layer(
        &mut self,
        visible: bool,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Box,
            style,
            layout,
            InteractionSnapshot {
                visible,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a component-owned visibility layer under an existing host.
    #[doc(hidden)]
    pub fn layer_node_under(
        &mut self,
        parent: NodeId,
        visible: bool,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let layer = self.layer(visible, style, layout, content);
        self.parents.pop();
        Some(layer)
    }
    pub fn scroll(
        &mut self,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> ScrollHandle {
        let node = self.mount(
            NodeKind::Scroll,
            style,
            layout,
            InteractionSnapshot {
                behavior: ControlBehavior::Scroll,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ScrollHandle {
            node,
            offset: Property::new(node, PropertyKind::ScrollOffset),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a component-owned scroll viewport under an existing host.
    #[doc(hidden)]
    pub fn scroll_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ScrollHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Scroll,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable,
                behavior: ControlBehavior::Scroll,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ScrollHandle {
            node,
            offset: Property::new(node, PropertyKind::ScrollOffset),
            style: Property::new(node, PropertyKind::Style),
        })
    }
    pub fn text(&mut self, content: impl AsRef<str>, color: ColorRgba8, size: f32) -> TextHandle {
        let content = self.ui.intern(content);
        let node = self.mount(
            NodeKind::Text,
            BoxStyle::default(),
            LayoutStyle::default(),
            InteractionSnapshot::default(),
        );
        self.ui.texts.insert(
            node,
            TextVisual {
                content,
                style: TextStyle {
                    color,
                    size,
                    line_height: size * 1.25,
                    family: StringId(1),
                    weight: 400,
                    align: TextAlign::Start,
                    vertical_align: crate::ui::TextAlign::Start,
                    fit_height: false,
                },
                revision: 1,
            },
        );
        TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a retained text node with node-owned reusable content storage.
    #[doc(hidden)]
    pub fn dynamic_text(
        &mut self,
        content: impl Into<String>,
        text_style: TextStyle,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> TextHandle {
        let node = self.mount(
            NodeKind::Text,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        let content = self.ui.allocate_dynamic_text(node, content.into());
        self.ui.texts.insert(
            node,
            TextVisual {
                content,
                style: text_style,
                revision: 1,
            },
        );
        let _ = self.ui.set_semantics(
            node,
            SemanticNode {
                role: SemanticRole::Text,
                name: SemanticName::Text(content),
                ..SemanticNode::default()
            },
        );
        TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        }
    }
    /// Creates a component-owned retained text node under an existing host while preserving the
    /// caller's complete visual, content revision, box style, and layout inputs.
    #[doc(hidden)]
    pub fn text_node_under(
        &mut self,
        parent: NodeId,
        visual: TextVisual,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        focusable: bool,
    ) -> Option<TextHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Text,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.pop();
        self.ui.texts.insert(node, visual);
        Some(TextHandle {
            node,
            text: Property::new(node, PropertyKind::Text),
            color: Property::new(node, PropertyKind::TextColor),
            enabled: Property::new(node, PropertyKind::Enabled),
            style: Property::new(node, PropertyKind::Style),
        })
    }
    pub fn button(
        &mut self,
        action: A,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let control = self.button_node(style, content);
        self.action_routes.push((control.node, action));
        control
    }
    /// Creates a foundation button node without storing a typed action in mounted UI. The
    /// component runtime uses this entry point so it can own generation-bound action factories.
    #[doc(hidden)]
    pub fn button_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle {
                main_axis_alignment: MainAxisAlignment::Center,
                cross_axis_alignment: CrossAxisAlignment::Center,
                ..LayoutStyle::default()
            },
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed toggle at the current mount parent.
    #[doc(hidden)]
    pub fn toggle_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Toggle,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed slider at the current mount parent.
    #[doc(hidden)]
    pub fn slider_node(
        &mut self,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Slider,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior: ControlBehavior::Value,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    /// Creates a runtime-routed foundation button under an already mounted component host.
    #[doc(hidden)]
    pub fn button_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let control = self.button_node(style, content);
        self.parents.pop();
        Some(control)
    }
    /// Creates a runtime-routed toggle under an already mounted component host.
    #[doc(hidden)]
    pub fn toggle_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        self.interactive_node_under(
            parent,
            NodeKind::Toggle,
            ControlBehavior::Activate,
            style,
            content,
        )
    }
    /// Creates a runtime-routed slider under an already mounted component host.
    #[doc(hidden)]
    pub fn slider_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        self.interactive_node_under(
            parent,
            NodeKind::Slider,
            ControlBehavior::Value,
            style,
            content,
        )
    }
    fn interactive_node_under(
        &mut self,
        parent: NodeId,
        kind: NodeKind,
        behavior: ControlBehavior,
        style: BoxStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            kind,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable: true,
                behavior,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a component-owned action node under an existing host.
    #[doc(hidden)]
    pub fn action_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        enabled: bool,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                enabled,
                focusable,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a noninteractive component-owned container under an existing host.
    #[doc(hidden)]
    pub fn container_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.container(style, layout, content);
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a platform-neutral text-input node under an existing component host.
    #[doc(hidden)]
    pub fn text_input_node_under(
        &mut self,
        parent: NodeId,
        style: BoxStyle,
        layout: LayoutStyle,
        enabled: bool,
        content: impl FnOnce(&mut Self),
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::TextInput,
            style,
            layout,
            InteractionSnapshot {
                enabled,
                focusable: true,
                behavior: ControlBehavior::TextInput,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        self.parents.pop();
        Some(ControlHandle::new(node))
    }
    /// Creates a component-owned action node with explicit tab-stop participation.
    #[doc(hidden)]
    pub fn action_node(
        &mut self,
        style: BoxStyle,
        focusable: bool,
        content: impl FnOnce(&mut Self),
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Button,
            style,
            LayoutStyle::default(),
            InteractionSnapshot {
                focusable,
                behavior: ControlBehavior::Activate,
                ..InteractionSnapshot::default()
            },
        );
        self.parents.push(node);
        content(self);
        self.parents.pop();
        ControlHandle::new(node)
    }
    pub fn image(&mut self, image: ImageId, style: BoxStyle) -> NodeId {
        let node = self.mount(
            NodeKind::Image,
            style,
            LayoutStyle::default(),
            InteractionSnapshot::default(),
        );
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint: None,
                content_version: 1,
            },
        );
        node
    }

    /// Creates a revisioned image node using complete composition layout inputs.
    pub fn dynamic_image(
        &mut self,
        image: ImageId,
        content_version: u64,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> ControlHandle {
        self.dynamic_image_tinted(image, content_version, None, style, layout)
    }

    /// Creates a revisioned image node with an optional alpha-mask tint.
    #[doc(hidden)]
    pub fn dynamic_image_tinted(
        &mut self,
        image: ImageId,
        content_version: u64,
        tint: Option<ColorRgba8>,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> ControlHandle {
        let node = self.mount(
            NodeKind::Image,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint,
                content_version: content_version.max(1),
            },
        );
        ControlHandle::new(node)
    }
    /// Creates a component-owned retained image under an existing host while preserving the
    /// caller's content version and layout inputs.
    #[doc(hidden)]
    pub fn image_node_under(
        &mut self,
        parent: NodeId,
        image: ImageId,
        content_version: u64,
        style: BoxStyle,
        layout: LayoutStyle,
    ) -> Option<ControlHandle> {
        if !self.ui.nodes.contains(parent) {
            return None;
        }
        self.parents.push(parent);
        let node = self.mount(
            NodeKind::Image,
            style,
            layout,
            InteractionSnapshot::default(),
        );
        self.parents.pop();
        self.ui.images.insert(
            node,
            ImageVisual {
                image,
                tint: None,
                content_version,
            },
        );
        Some(ControlHandle::new(node))
    }
    pub fn semantic(
        &mut self,
        node: NodeId,
        label: impl AsRef<str>,
        role: SemanticRole,
    ) -> Result<bool, SemanticError> {
        let label = self.ui.intern(label);
        self.ui
            .set_semantics(node, SemanticNode::named(role, label))
    }
    /// Attaches a complete component-authored semantic record during mount.
    pub fn semantic_node(
        &mut self,
        node: NodeId,
        semantic: SemanticNode,
    ) -> Result<bool, SemanticError> {
        self.ui.set_semantics(node, semantic)
    }
    /// Attaches a shell-understood role without coupling the node to shell-specific paint.
    #[doc(hidden)]
    pub fn window_chrome_role(
        &mut self,
        node: NodeId,
        role: Option<crate::shell::window_chrome::WindowChromeRole>,
    ) -> bool {
        self.ui.set_window_chrome_role(node, role)
    }
    /// Attaches hit-test tuning to a shell-understood chrome region during mount.
    #[doc(hidden)]
    pub fn window_chrome_hit_spec(
        &mut self,
        node: NodeId,
        spec: Option<crate::shell::window_chrome::WindowChromeHitSpec>,
    ) -> bool {
        self.ui.set_window_chrome_hit_spec(node, spec)
    }
    /// Attaches a semantic pointer request during mount.
    #[doc(hidden)]
    pub fn pointer_request(
        &mut self,
        node: NodeId,
        request: Option<crate::assets::PointerRequest>,
    ) -> bool {
        self.ui.set_pointer_request(node, request)
    }
    pub fn disabled(&mut self, node: NodeId, disabled: bool) -> bool {
        self.ui.set_disabled(node, disabled)
    }
    pub fn read_only(&mut self, node: NodeId, read_only: bool) -> bool {
        self.ui.set_read_only(node, read_only)
    }
    pub fn busy(&mut self, node: NodeId, busy: bool) -> bool {
        self.ui.set_busy(node, busy)
    }
    pub fn checked(&mut self, node: NodeId, checked: bool) -> bool {
        self.ui.set_checked(node, checked)
    }
    #[doc(hidden)]
    pub fn control_value(&mut self, node: NodeId, value: f32) -> bool {
        self.ui.set_control_value(node, value)
    }
    pub fn mixed(&mut self, node: NodeId, mixed: bool) -> bool {
        self.ui.set_mixed(node, mixed)
    }
    pub fn selected(&mut self, node: NodeId, selected: bool) -> bool {
        self.ui.set_selected(node, selected)
    }
    pub fn expanded(&mut self, node: NodeId, expanded: bool) -> bool {
        self.ui.set_expanded(node, expanded)
    }
    pub fn invalid(&mut self, node: NodeId, invalid: bool) -> bool {
        self.ui.set_invalid(node, invalid)
    }
    pub fn active(&mut self, node: NodeId, active: bool) -> bool {
        self.ui.set_active(node, active)
    }
    pub fn highlighted(&mut self, node: NodeId, highlighted: bool) -> bool {
        self.ui.set_highlighted(node, highlighted)
    }
    /// Registers explicit default behavior for an advanced/custom foundation node.
    pub fn control_behavior(&mut self, node: NodeId, behavior: ControlBehavior) -> bool {
        self.ui.set_control_behavior(node, behavior)
    }
    /// Registers an opt-in custom or first-party component style binding.
    pub fn style_binding(&mut self, binding: StyleBinding) -> bool {
        self.ui.register_style_binding(binding)
    }
    pub(crate) fn layout_style(&mut self, node: NodeId, layout: LayoutStyle) -> bool {
        self.ui.set_layout_style(node, layout)
    }
    pub fn style_id(&mut self, node: NodeId, style: ComponentStyleId) -> bool {
        self.ui.set_style_id(node, style)
    }
    pub fn hover_within(&mut self, node: NodeId, enabled: bool) -> bool {
        self.ui.set_hover_within(node, enabled)
    }
    /// Enables visual pointer states without activation, capture, or focus behavior.
    pub fn visual_interaction(&mut self, node: NodeId, hover: bool, press: bool) -> bool {
        self.ui.set_visual_interaction(node, hover, press)
    }
    pub(crate) fn visual_style(
        &mut self,
        node: NodeId,
        style: Option<Arc<crate::theme::CompiledComponentStyle>>,
        own: bool,
        overlay: bool,
    ) -> bool {
        self.ui.set_visual_style(node, style, own, overlay)
    }
    pub fn style_override(
        &mut self,
        node: NodeId,
        slot: StyleSlotId,
        patch: StylePropertyPatch,
    ) -> bool {
        self.ui.set_style_override(node, slot, patch)
    }
    pub fn listen(&mut self, node: NodeId, mask: u16) {
        self.ui.set_listener_mask(node, mask);
    }
    /// Associates an interactive value control with the node that owns its spatial track.
    #[doc(hidden)]
    pub fn value_track(&mut self, node: NodeId, track: NodeId, axis: ValueAxis) -> bool {
        if !self.ui.nodes.contains(node) || !self.ui.nodes.contains(track) {
            return false;
        }
        let Some(interaction) = self.ui.interactions.get_mut(node) else {
            return false;
        };
        if interaction.value_track == Some(track) && interaction.value_axis == Some(axis) {
            return false;
        }
        interaction.value_track = Some(track);
        interaction.value_axis = Some(axis);
        true
    }
    fn mount(
        &mut self,
        kind: NodeKind,
        style: BoxStyle,
        layout: LayoutStyle,
        mut interaction: InteractionSnapshot,
    ) -> NodeId {
        let parent = self.parents.last().copied();
        let node = self
            .ui
            .nodes
            .spawn(parent)
            .expect("mounted node arena exhausted");
        self.ui.kinds.insert(node, kind);
        if style != BoxStyle::default() {
            self.ui.box_styles.insert(node, style);
        }
        if layout != LayoutStyle::default() {
            self.ui.layouts.insert(node, layout);
        }
        interaction
            .flags
            .set(InteractionFlags::DISABLED, !interaction.enabled);
        if interaction != InteractionSnapshot::default() {
            self.ui.interactions.insert(node, interaction);
            if let Some(core) = self.ui.nodes.core_mut(node) {
                core.state_bits = interaction.flags.bits();
            }
        }
        // Nested controls own their state. Inheriting a scroll viewport's state here
        // leaves a second foundation binding competing with the control's inline style.
        let state_root = if interaction.behavior != ControlBehavior::None {
            node
        } else {
            parent.and_then(|parent| self.ui.nearest_control(parent)).unwrap_or(node)
        };
        let component = match kind {
            NodeKind::Box => "box",
            NodeKind::Text => "text",
            NodeKind::Image => "image",
            NodeKind::Button => "button",
            NodeKind::Toggle => "toggle",
            NodeKind::TextInput => "text-input",
            NodeKind::Slider => "slider",
            NodeKind::Scroll => "scroll",
            NodeKind::Collection => "collection",
            NodeKind::Custom(_) => "custom",
        };
        self.ui.register_style_binding(
            StyleBinding::new(
                state_root,
                ThemeScopeId::new(0, 1),
                ComponentStyleId::named(ThemeDomainId::APPLICATION, component, "default"),
            )
            .slot(StyleSlotId::named("root"), node),
        );
        node
    }
}

//! Application-domain mount seam over the neutral overlay lifecycle owner.

use std::fmt;

use crate::runtime::Ui;
use crate::ui::OverlayHost as NeutralOverlayHost;
use crate::ui::{
    BoxStyle, DismissReason, Flow, LayoutStyle, MountedUi, OverlayCloseOutcome, OverlayDiagnostics,
    OverlayDismissResult, OverlayEntry, OverlayError, OverlayId, OverlayOpenRequest, OverlayOpened,
    SizeRule, UiNodeId,
};

/// Mounted, noninteractive portal target for one application's overlay stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationOverlayHostRef {
    node: UiNodeId,
}

impl ApplicationOverlayHostRef {
    /// The mounted host node itself.
    pub const fn node(self) -> UiNodeId {
        self.node
    }

    /// Visual target for runtime-owned portal children.
    pub const fn portal_host(self) -> UiNodeId {
        self.node
    }
}

/// Application wrapper that binds one neutral overlay lifecycle owner to one mounted view.
#[derive(Default)]
pub struct ApplicationOverlayHost {
    lifecycle: NeutralOverlayHost,
    mounted: Option<ApplicationOverlayHostRef>,
}

impl fmt::Debug for ApplicationOverlayHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationOverlayHost")
            .field("mounted", &self.mounted)
            .field("entry_count", &self.lifecycle.entries().len())
            .field("background_is_inert", &self.lifecycle.background_is_inert())
            .field("diagnostics", &self.lifecycle.diagnostics())
            .finish()
    }
}

impl ApplicationOverlayHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn mounted(&self) -> Option<ApplicationOverlayHostRef> {
        self.mounted
    }

    /// Mounts the one full-view, noninteractive portal target owned by this host instance.
    pub fn mount<Action: 'static>(
        &mut self,
        ui: &mut Ui<'_, '_, Action>,
        parent: UiNodeId,
    ) -> Result<ApplicationOverlayHostRef, ApplicationOverlayHostError> {
        if let Some(mounted) = self.mounted {
            return Err(ApplicationOverlayHostError::AlreadyMounted(mounted));
        }

        let style = BoxStyle {
            width: SizeRule::Fill(1.0),
            height: SizeRule::Fill(1.0),
            ..BoxStyle::default()
        };
        let layout = LayoutStyle {
            flow: Flow::Overlay,
            contain: true,
            ..LayoutStyle::default()
        };
        let node = ui
            .foundation()
            .container_node_under(parent, style, layout, |_| {})
            .ok_or(ApplicationOverlayHostError::StaleVisualParent(parent))?
            .node;
        let mounted = ApplicationOverlayHostRef { node };
        self.mounted = Some(mounted);
        Ok(mounted)
    }

    /// Returns whether this host's exact mounted generation belongs to `ui`.
    pub fn is_mounted_on(&self, ui: &MountedUi) -> bool {
        self.mounted
            .is_some_and(|mounted| ui.nodes.contains(mounted.node))
    }

    pub fn diagnostics(&self) -> OverlayDiagnostics {
        self.lifecycle.diagnostics()
    }

    /// Entries in the neutral owner's bottommost-to-topmost order.
    pub fn entries(&self) -> &[OverlayEntry] {
        self.lifecycle.entries()
    }

    pub fn entry(&self, id: OverlayId) -> Option<&OverlayEntry> {
        self.lifecycle.entry(id)
    }

    pub fn top(&self) -> Option<&OverlayEntry> {
        self.lifecycle.top()
    }

    pub fn active_modal(&self) -> Option<OverlayId> {
        self.lifecycle.active_modal()
    }

    pub fn background_is_inert(&self) -> bool {
        self.lifecycle.background_is_inert()
    }

    /// Opens through the neutral owner after proving this is the same live mounted UI.
    pub fn open(
        &mut self,
        ui: &MountedUi,
        request: OverlayOpenRequest,
    ) -> Result<OverlayOpened, ApplicationOverlayHostError> {
        self.require_live_mount(ui)?;
        self.lifecycle
            .open(ui, request)
            .map_err(ApplicationOverlayHostError::Lifecycle)
    }

    pub fn dismiss(
        &mut self,
        id: OverlayId,
        reason: DismissReason,
    ) -> Result<OverlayDismissResult, ApplicationOverlayHostError> {
        self.lifecycle
            .dismiss(id, reason)
            .map_err(ApplicationOverlayHostError::Lifecycle)
    }

    /// Delegates forced anchor cleanup to the neutral lifecycle owner.
    pub fn anchor_removed(&mut self, node: UiNodeId) -> Vec<OverlayCloseOutcome> {
        self.lifecycle.anchor_removed(node)
    }

    /// Closes the current stack for view loss while retaining the mounted host generation.
    pub fn view_lost(&mut self) -> OverlayCloseOutcome {
        self.lifecycle.close_all(DismissReason::ViewLost)
    }

    /// Closes the current stack and releases this wrapper's mounted association.
    pub fn unmount(&mut self) -> OverlayCloseOutcome {
        let outcome = self.lifecycle.close_all(DismissReason::OwnerUnmounted);
        self.mounted = None;
        outcome
    }

    fn require_live_mount(
        &self,
        ui: &MountedUi,
    ) -> Result<ApplicationOverlayHostRef, ApplicationOverlayHostError> {
        let mounted = self
            .mounted
            .ok_or(ApplicationOverlayHostError::NotMounted)?;
        if !ui.nodes.contains(mounted.node) {
            return Err(ApplicationOverlayHostError::StaleMount(mounted));
        }
        Ok(mounted)
    }
}

/// Mount-association errors remain distinct from neutral lifecycle validation errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOverlayHostError {
    AlreadyMounted(ApplicationOverlayHostRef),
    NotMounted,
    StaleMount(ApplicationOverlayHostRef),
    StaleVisualParent(UiNodeId),
    Lifecycle(OverlayError),
}

impl fmt::Display for ApplicationOverlayHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyMounted(mounted) => {
                write!(
                    formatter,
                    "overlay host is already mounted at {:?}",
                    mounted.node
                )
            }
            Self::NotMounted => formatter.write_str("overlay host is not mounted"),
            Self::StaleMount(mounted) => {
                write!(formatter, "overlay host mount {:?} is stale", mounted.node)
            }
            Self::StaleVisualParent(parent) => {
                write!(formatter, "overlay visual parent {parent:?} is stale")
            }
            Self::Lifecycle(error) => write!(formatter, "overlay lifecycle rejected: {error:?}"),
        }
    }
}

impl std::error::Error for ApplicationOverlayHostError {}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::runtime::{
        Component, ComponentRuntimeDriver, CreateContext, UpdateContext, ViewRuntime,
    };
    use crate::ui::{
        NodeKind, OutsidePressPolicy, OverlayDismissPolicy, OverlayFocusContainment,
        OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayInitialFocus,
        OverlayModality, UiRoot,
    };

    use super::*;

    struct MountedHost {
        host: Rc<RefCell<ApplicationOverlayHost>>,
        mounted: Rc<Cell<Option<(ApplicationOverlayHostRef, UiNodeId)>>>,
        duplicate_rejected: Rc<Cell<bool>>,
    }

    impl Component for MountedHost {
        type State = ();
        type Action = ();

        fn create(&self, _context: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let mounted = self.host.borrow_mut().mount(ui, root.0).unwrap();
            self.mounted.set(Some((mounted, root.0)));
            self.duplicate_rejected.set(matches!(
                self.host.borrow_mut().mount(ui, root.0),
                Err(ApplicationOverlayHostError::AlreadyMounted(candidate)) if candidate == mounted
            ));
            root
        }

        fn action(
            &self,
            _state: &mut Self::State,
            _action: Self::Action,
            _context: &mut UpdateContext<'_, Self>,
        ) {
        }
    }

    struct MountedHostHarness {
        runtime: ViewRuntime<ComponentRuntimeDriver<MountedHost>>,
        host: Rc<RefCell<ApplicationOverlayHost>>,
        mounted: ApplicationOverlayHostRef,
        anchor: UiNodeId,
        duplicate_rejected: Rc<Cell<bool>>,
    }

    fn mounted_host() -> MountedHostHarness {
        let host = Rc::new(RefCell::new(ApplicationOverlayHost::new()));
        let mounted = Rc::new(Cell::new(None));
        let duplicate_rejected = Rc::new(Cell::new(false));
        let runtime = ViewRuntime::from_component(MountedHost {
            host: host.clone(),
            mounted: mounted.clone(),
            duplicate_rejected: duplicate_rejected.clone(),
        })
        .unwrap();
        let (reference, root) = mounted.get().unwrap();
        MountedHostHarness {
            runtime,
            host,
            mounted: reference,
            anchor: root,
            duplicate_rejected,
        }
    }

    fn menu(anchor: UiNodeId) -> OverlayOpenRequest {
        OverlayOpenRequest {
            anchor: crate::ui::OverlayAnchor::Node(anchor),
            parent: None,
            modality: OverlayModality::NonModal,
            dismissal: OverlayDismissPolicy {
                escape: true,
                outside_press: OutsidePressPolicy::DismissAndConsume,
                focus_lost: false,
                pointer_departure: false,
            },
            focus: OverlayFocusLifecycle {
                initial: OverlayInitialFocus::FirstFocusable,
                containment: OverlayFocusContainment::None,
                restoration: OverlayFocusRestoration::TargetThenNearest(anchor),
            },
        }
    }

    #[test]
    fn mount_creates_one_noninteractive_full_view_portal_target() {
        let harness = mounted_host();
        assert!(harness.duplicate_rejected.get());
        assert!(harness.host.borrow().is_mounted_on(harness.runtime.ui()));
        assert_eq!(
            harness.runtime.ui().kinds.get(harness.mounted.node()),
            Some(&NodeKind::Box)
        );
        let style = harness
            .runtime
            .ui()
            .box_styles
            .get(harness.mounted.node())
            .unwrap();
        assert_eq!(style.width, SizeRule::Fill(1.0));
        assert_eq!(style.height, SizeRule::Fill(1.0));
        let layout = harness
            .runtime
            .ui()
            .layouts
            .get(harness.mounted.node())
            .unwrap();
        assert_eq!(layout.flow, Flow::Overlay);
        assert!(layout.contain);
        assert!(
            harness
                .runtime
                .ui()
                .interactions
                .get(harness.mounted.node())
                .is_none()
        );
        assert!(
            harness
                .runtime
                .ui()
                .semantics
                .get(harness.mounted.node())
                .is_none()
        );
        assert_eq!(harness.mounted.portal_host(), harness.mounted.node());
    }

    #[test]
    fn opens_only_against_its_live_ui_and_delegates_modal_state() {
        let harness = mounted_host();
        let mut request = menu(harness.anchor);
        request.modality = OverlayModality::Modal;
        request.focus.containment = OverlayFocusContainment::Contain;
        let opened = harness
            .host
            .borrow_mut()
            .open(harness.runtime.ui(), request)
            .unwrap();
        assert_eq!(
            opened.focus,
            OverlayFocusRequest::Initial(OverlayInitialFocus::FirstFocusable)
        );
        assert_eq!(harness.host.borrow().active_modal(), Some(opened.id));
        assert!(harness.host.borrow().background_is_inert());
        assert_eq!(harness.host.borrow().entries().len(), 1);

        let foreign = MountedUi::default();
        assert!(matches!(
            harness
                .host
                .borrow_mut()
                .open(&foreign, menu(harness.anchor)),
            Err(ApplicationOverlayHostError::StaleMount(_))
        ));
        assert_eq!(harness.host.borrow().diagnostics().failures, 0);
    }

    #[test]
    fn nested_dismissal_preserves_neutral_order_focus_and_input_effects() {
        let harness = mounted_host();
        let parent = harness
            .host
            .borrow_mut()
            .open(harness.runtime.ui(), menu(harness.anchor))
            .unwrap()
            .id;
        let mut child_request = menu(harness.anchor);
        child_request.parent = Some(parent);
        let child = harness
            .host
            .borrow_mut()
            .open(harness.runtime.ui(), child_request)
            .unwrap()
            .id;
        let OverlayDismissResult::Dismissed(outcome) = harness
            .host
            .borrow_mut()
            .dismiss(parent, DismissReason::OutsidePress)
            .unwrap()
        else {
            panic!("outside press should dismiss the subtree")
        };
        assert_eq!(
            outcome
                .dismissed
                .iter()
                .map(|dismissed| dismissed.id)
                .collect::<Vec<_>>(),
            vec![child, parent]
        );
        assert!(outcome.consume_input);
        assert_eq!(
            outcome.focus,
            OverlayFocusRequest::Restore {
                target: harness.anchor,
                nearest_fallback: true,
            }
        );
        assert!(harness.host.borrow().entries().is_empty());
    }

    #[test]
    fn explicit_unmount_closes_entries_and_releases_the_mount_association() {
        let harness = mounted_host();
        let id = harness
            .host
            .borrow_mut()
            .open(harness.runtime.ui(), menu(harness.anchor))
            .unwrap()
            .id;
        let outcome = harness.host.borrow_mut().unmount();
        assert_eq!(outcome.dismissed.len(), 1);
        assert_eq!(outcome.dismissed[0].id, id);
        assert_eq!(outcome.dismissed[0].reason, DismissReason::OwnerUnmounted);
        assert_eq!(harness.host.borrow().mounted(), None);
        assert!(matches!(
            harness
                .host
                .borrow_mut()
                .open(harness.runtime.ui(), menu(harness.anchor)),
            Err(ApplicationOverlayHostError::NotMounted)
        ));
        assert!(harness.runtime.ui().nodes.contains(harness.mounted.node()));
    }
}

//! Environment-derived presentation planning over stable [`Scaffold`] slots.

use std::fmt;

use crate::components::application::primitives::{EnvironmentRevision, EnvironmentSnapshot, InputCapabilities};
use crate::runtime::{RuntimeResult, Ui};
use crate::ui::{Flow, LayoutStyle, MountWriter, UiNodeId};

use super::{Scaffold, ScaffoldRef, ScaffoldSlot, ScaffoldStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdaptiveWidthClass {
    Compact,
    Medium,
    Expanded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdaptiveSlotPresentation {
    NavigationRail,
    NavigationBar,
    TopBar,
    PrimaryContent,
    SecondaryAlongside,
    SecondaryRoute,
    SecondarySheet,
    StatusBand,
    FloatingAction,
    Overlay,
}

impl AdaptiveSlotPresentation {
    pub const fn description(self) -> &'static str {
        match self {
            Self::NavigationRail => "navigation rail",
            Self::NavigationBar => "navigation bar",
            Self::TopBar => "top bar",
            Self::PrimaryContent => "primary content",
            Self::SecondaryAlongside => "secondary content alongside",
            Self::SecondaryRoute => "secondary content route",
            Self::SecondarySheet => "secondary content sheet",
            Self::StatusBand => "status band",
            Self::FloatingAction => "floating action",
            Self::Overlay => "application overlay",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AdaptiveSlotPlan {
    slot: ScaffoldSlot,
    presentation: AdaptiveSlotPresentation,
}

impl AdaptiveSlotPlan {
    pub const fn slot(self) -> ScaffoldSlot {
        self.slot
    }

    pub const fn presentation(self) -> AdaptiveSlotPresentation {
        self.presentation
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdaptiveScaffoldPlan {
    environment_revision: EnvironmentRevision,
    width_class: AdaptiveWidthClass,
    effective_width: f32,
    touch_primary: bool,
    slots: Vec<AdaptiveSlotPlan>,
}

impl AdaptiveScaffoldPlan {
    pub const fn environment_revision(&self) -> EnvironmentRevision {
        self.environment_revision
    }

    pub const fn width_class(&self) -> AdaptiveWidthClass {
        self.width_class
    }

    pub const fn effective_width(&self) -> f32 {
        self.effective_width
    }

    pub const fn touch_primary(&self) -> bool {
        self.touch_primary
    }

    pub fn slots(&self) -> &[AdaptiveSlotPlan] {
        &self.slots
    }

    pub fn presentation(&self, slot: ScaffoldSlot) -> Option<AdaptiveSlotPresentation> {
        self.slots
            .iter()
            .find(|entry| entry.slot == slot)
            .map(|entry| entry.presentation)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveScaffoldPolicy {
    compact_breakpoint: f32,
    expanded_breakpoint: f32,
}

impl AdaptiveScaffoldPolicy {
    pub fn new(
        compact_breakpoint: f32,
        expanded_breakpoint: f32,
    ) -> Result<Self, AdaptiveScaffoldPolicyError> {
        if !compact_breakpoint.is_finite() || compact_breakpoint <= 0.0 {
            return Err(AdaptiveScaffoldPolicyError::InvalidCompactBreakpoint);
        }
        if !expanded_breakpoint.is_finite() || expanded_breakpoint <= compact_breakpoint {
            return Err(AdaptiveScaffoldPolicyError::InvalidExpandedBreakpoint);
        }
        Ok(Self {
            compact_breakpoint,
            expanded_breakpoint,
        })
    }

    pub const fn compact_breakpoint(self) -> f32 {
        self.compact_breakpoint
    }

    pub const fn expanded_breakpoint(self) -> f32 {
        self.expanded_breakpoint
    }

    pub fn resolve(
        self,
        scaffold: &Scaffold,
        environment: &EnvironmentSnapshot,
    ) -> AdaptiveScaffoldPlan {
        let values = environment.values();
        let constrained_width = values
            .constraints
            .horizontal
            .max
            .map_or(values.available_size.width, |maximum| {
                values.available_size.width.min(maximum)
            });
        let effective_width = constrained_width / values.text_scale;
        let width_class = if effective_width < self.compact_breakpoint {
            AdaptiveWidthClass::Compact
        } else if effective_width < self.expanded_breakpoint {
            AdaptiveWidthClass::Medium
        } else {
            AdaptiveWidthClass::Expanded
        };
        let capabilities = values.input_capabilities;
        let touch_primary = capabilities.contains(InputCapabilities::TOUCH)
            && !capabilities.intersects(InputCapabilities::MOUSE | InputCapabilities::HOVER);
        let slots = scaffold
            .slots()
            .iter()
            .map(|spec| AdaptiveSlotPlan {
                slot: spec.slot(),
                presentation: presentation_for(spec.slot(), width_class, touch_primary),
            })
            .collect();
        AdaptiveScaffoldPlan {
            environment_revision: environment.revision(),
            width_class,
            effective_width,
            touch_primary,
            slots,
        }
    }
}

impl Default for AdaptiveScaffoldPolicy {
    fn default() -> Self {
        Self {
            compact_breakpoint: 600.0,
            expanded_breakpoint: 1_024.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptiveScaffoldPolicyError {
    InvalidCompactBreakpoint,
    InvalidExpandedBreakpoint,
}

impl fmt::Display for AdaptiveScaffoldPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid adaptive scaffold policy: {self:?}")
    }
}

impl std::error::Error for AdaptiveScaffoldPolicyError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveScaffoldStyle {
    pub compact: ScaffoldStyle,
    pub medium: ScaffoldStyle,
    pub expanded: ScaffoldStyle,
}

impl AdaptiveScaffoldStyle {
    const fn for_class(self, width_class: AdaptiveWidthClass) -> ScaffoldStyle {
        match width_class {
            AdaptiveWidthClass::Compact => self.compact,
            AdaptiveWidthClass::Medium => self.medium,
            AdaptiveWidthClass::Expanded => self.expanded,
        }
    }
}

impl Default for AdaptiveScaffoldStyle {
    fn default() -> Self {
        let compact = ScaffoldStyle::default();
        let wider = ScaffoldStyle {
            layout: LayoutStyle {
                flow: Flow::Horizontal,
                ..LayoutStyle::default()
            },
            ..ScaffoldStyle::default()
        };
        Self {
            compact,
            medium: wider,
            expanded: wider,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdaptiveScaffold {
    scaffold: Scaffold,
    policy: AdaptiveScaffoldPolicy,
    style: AdaptiveScaffoldStyle,
}

impl AdaptiveScaffold {
    pub fn new(scaffold: Scaffold) -> Self {
        Self {
            scaffold,
            policy: AdaptiveScaffoldPolicy::default(),
            style: AdaptiveScaffoldStyle::default(),
        }
    }

    pub const fn policy(mut self, policy: AdaptiveScaffoldPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub const fn style(mut self, style: AdaptiveScaffoldStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn scaffold(&self) -> &Scaffold {
        &self.scaffold
    }

    pub fn plan(&self, environment: &EnvironmentSnapshot) -> AdaptiveScaffoldPlan {
        self.policy.resolve(&self.scaffold, environment)
    }

    pub fn mount<'storage, Action, Content>(
        &self,
        ui: &mut Ui<'_, 'storage, Action>,
        host: UiNodeId,
        environment: &EnvironmentSnapshot,
        content: Content,
    ) -> RuntimeResult<AdaptiveScaffoldRef>
    where
        Action: 'static,
        Content: FnMut(ScaffoldSlot, &mut MountWriter<'storage, Action>),
    {
        let plan = self.plan(environment);
        let scaffold = self
            .scaffold
            .clone()
            .style(self.style.for_class(plan.width_class));
        let mounted = scaffold.mount(ui, host, content)?;
        Ok(AdaptiveScaffoldRef {
            scaffold: mounted,
            plan,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveSlotTransition {
    slot: ScaffoldSlot,
    node: UiNodeId,
    from: AdaptiveSlotPresentation,
    to: AdaptiveSlotPresentation,
}

impl AdaptiveSlotTransition {
    pub const fn slot(self) -> ScaffoldSlot {
        self.slot
    }

    pub const fn node(self) -> UiNodeId {
        self.node
    }

    pub const fn from(self) -> AdaptiveSlotPresentation {
        self.from
    }

    pub const fn to(self) -> AdaptiveSlotPresentation {
        self.to
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdaptiveScaffoldTransition {
    from_revision: EnvironmentRevision,
    to_revision: EnvironmentRevision,
    changes: Vec<AdaptiveSlotTransition>,
}

impl AdaptiveScaffoldTransition {
    pub const fn from_revision(&self) -> EnvironmentRevision {
        self.from_revision
    }

    pub const fn to_revision(&self) -> EnvironmentRevision {
        self.to_revision
    }

    pub fn changes(&self) -> &[AdaptiveSlotTransition] {
        &self.changes
    }

    pub fn is_unchanged(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct AdaptiveScaffoldRef {
    scaffold: ScaffoldRef,
    plan: AdaptiveScaffoldPlan,
}

impl AdaptiveScaffoldRef {
    pub const fn node(&self) -> UiNodeId {
        self.scaffold.node()
    }

    pub const fn scaffold(&self) -> &ScaffoldRef {
        &self.scaffold
    }

    pub const fn plan(&self) -> &AdaptiveScaffoldPlan {
        &self.plan
    }

    /// Accepts a new presentation plan and reports changes against the same mounted slot nodes.
    /// Applying layout or replacing navigation/overlay components remains caller-owned.
    pub fn reconcile_plan(
        &mut self,
        next: AdaptiveScaffoldPlan,
    ) -> Result<AdaptiveScaffoldTransition, AdaptiveScaffoldError> {
        if self.plan.slots.len() != next.slots.len()
            || self
                .plan
                .slots
                .iter()
                .zip(&next.slots)
                .any(|(before, after)| before.slot != after.slot)
        {
            return Err(AdaptiveScaffoldError::SlotSetChanged);
        }
        let mut changes = Vec::new();
        for (before, after) in self.plan.slots.iter().zip(&next.slots) {
            if before.presentation != after.presentation {
                let node = self
                    .scaffold
                    .slot(before.slot)
                    .ok_or(AdaptiveScaffoldError::MountedSlotMissing(before.slot))?
                    .node();
                changes.push(AdaptiveSlotTransition {
                    slot: before.slot,
                    node,
                    from: before.presentation,
                    to: after.presentation,
                });
            }
        }
        let transition = AdaptiveScaffoldTransition {
            from_revision: self.plan.environment_revision,
            to_revision: next.environment_revision,
            changes,
        };
        self.plan = next;
        Ok(transition)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptiveScaffoldError {
    SlotSetChanged,
    MountedSlotMissing(ScaffoldSlot),
}

impl fmt::Display for AdaptiveScaffoldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid adaptive scaffold transition: {self:?}")
    }
}

impl std::error::Error for AdaptiveScaffoldError {}

fn presentation_for(
    slot: ScaffoldSlot,
    width_class: AdaptiveWidthClass,
    touch_primary: bool,
) -> AdaptiveSlotPresentation {
    match slot {
        ScaffoldSlot::Navigation => {
            if width_class == AdaptiveWidthClass::Compact
                || (width_class == AdaptiveWidthClass::Medium && touch_primary)
            {
                AdaptiveSlotPresentation::NavigationBar
            } else {
                AdaptiveSlotPresentation::NavigationRail
            }
        }
        ScaffoldSlot::Top => AdaptiveSlotPresentation::TopBar,
        ScaffoldSlot::Content => AdaptiveSlotPresentation::PrimaryContent,
        ScaffoldSlot::Secondary => match width_class {
            AdaptiveWidthClass::Compact => AdaptiveSlotPresentation::SecondaryRoute,
            AdaptiveWidthClass::Medium => AdaptiveSlotPresentation::SecondarySheet,
            AdaptiveWidthClass::Expanded => AdaptiveSlotPresentation::SecondaryAlongside,
        },
        ScaffoldSlot::Status => AdaptiveSlotPresentation::StatusBand,
        ScaffoldSlot::FloatingAction => AdaptiveSlotPresentation::FloatingAction,
        ScaffoldSlot::Overlay => AdaptiveSlotPresentation::Overlay,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::components::application::primitives::{EnvironmentState, EnvironmentValues};
    use crate::foundation::SizeF;
    use crate::runtime::{Component, CreateContext, UpdateContext, ViewRuntime};
    use crate::ui::{BoxStyle, SemanticRole, UiRoot};

    use super::*;
    use crate::components::application::ScaffoldSlotSpec;

    fn scaffold() -> Scaffold {
        Scaffold::new(
            "Editor",
            [
                ScaffoldSlotSpec::new(ScaffoldSlot::Navigation, "Files").unwrap(),
                ScaffoldSlotSpec::new(ScaffoldSlot::Content, "Editor").unwrap(),
                ScaffoldSlotSpec::new(ScaffoldSlot::Secondary, "Inspector").unwrap(),
            ],
        )
        .unwrap()
    }

    fn environment(
        width: f32,
        text_scale: f32,
        capabilities: InputCapabilities,
    ) -> EnvironmentSnapshot {
        EnvironmentState::new(EnvironmentValues {
            available_size: SizeF {
                width,
                height: 800.0,
            },
            text_scale,
            input_capabilities: capabilities,
            ..EnvironmentValues::default()
        })
        .unwrap()
        .snapshot()
    }

    #[test]
    fn policy_uses_effective_width_and_touch_primary_input_deterministically() {
        let adaptive = AdaptiveScaffold::new(scaffold());
        let compact = adaptive.plan(&environment(900.0, 2.0, InputCapabilities::MOUSE));
        assert_eq!(compact.width_class(), AdaptiveWidthClass::Compact);
        assert_eq!(compact.effective_width(), 450.0);
        assert_eq!(
            compact.presentation(ScaffoldSlot::Navigation),
            Some(AdaptiveSlotPresentation::NavigationBar)
        );
        assert_eq!(
            compact.presentation(ScaffoldSlot::Secondary),
            Some(AdaptiveSlotPresentation::SecondaryRoute)
        );

        let touch = adaptive.plan(&environment(800.0, 1.0, InputCapabilities::TOUCH));
        assert_eq!(touch.width_class(), AdaptiveWidthClass::Medium);
        assert!(touch.touch_primary());
        assert_eq!(
            touch.presentation(ScaffoldSlot::Navigation),
            Some(AdaptiveSlotPresentation::NavigationBar)
        );

        let expanded = adaptive.plan(&environment(1_400.0, 1.0, InputCapabilities::MOUSE));
        assert_eq!(expanded.width_class(), AdaptiveWidthClass::Expanded);
        assert_eq!(
            expanded.presentation(ScaffoldSlot::Secondary),
            Some(AdaptiveSlotPresentation::SecondaryAlongside)
        );
    }

    struct Fixture {
        reference: Rc<RefCell<Option<AdaptiveScaffoldRef>>>,
        environment: EnvironmentSnapshot,
    }

    impl Component for Fixture {
        type State = ();
        type Action = ();

        fn create(&self, _: &mut CreateContext<'_>) -> Self::State {}

        fn mount(&self, _: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            let root = ui
                .foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {});
            let reference = AdaptiveScaffold::new(scaffold())
                .mount(ui, root.0, &self.environment, |_, _| {})
                .unwrap();
            *self.reference.borrow_mut() = Some(reference);
            root
        }

        fn action(&self, _: &mut Self::State, _: Self::Action, _: &mut UpdateContext<'_, Self>) {}
    }

    #[test]
    fn mounted_transition_reports_new_presentations_against_the_same_slot_nodes() {
        let reference = Rc::new(RefCell::new(None));
        let mut environment_state = EnvironmentState::new(EnvironmentValues {
            available_size: SizeF {
                width: 1_400.0,
                height: 800.0,
            },
            input_capabilities: InputCapabilities::MOUSE,
            ..EnvironmentValues::default()
        })
        .unwrap();
        let runtime = ViewRuntime::from_component(Fixture {
            reference: reference.clone(),
            environment: environment_state.snapshot(),
        })
        .unwrap();
        let mut reference = reference.borrow_mut().take().unwrap();
        let navigation_node = reference
            .scaffold()
            .slot(ScaffoldSlot::Navigation)
            .unwrap()
            .node();
        assert_eq!(
            runtime.ui().semantics.get(navigation_node).unwrap().role,
            SemanticRole::Navigation
        );

        let next_values = EnvironmentValues {
            available_size: SizeF {
                width: 500.0,
                height: 800.0,
            },
            input_capabilities: InputCapabilities::TOUCH,
            ..EnvironmentValues::default()
        };
        let next_environment = environment_state.update(next_values).unwrap().snapshot;
        let next = AdaptiveScaffold::new(scaffold()).plan(&next_environment);
        let transition = reference.reconcile_plan(next).unwrap();
        assert_eq!(transition.from_revision().get(), 1);
        assert_eq!(transition.to_revision().get(), 2);
        let navigation = transition
            .changes()
            .iter()
            .find(|change| change.slot() == ScaffoldSlot::Navigation)
            .unwrap();
        assert_eq!(navigation.node(), navigation_node);
        assert_eq!(navigation.from(), AdaptiveSlotPresentation::NavigationRail);
        assert_eq!(navigation.to(), AdaptiveSlotPresentation::NavigationBar);
        assert!(runtime.ui().kinds.get(navigation_node).is_some());
    }
}

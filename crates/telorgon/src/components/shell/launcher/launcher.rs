//! Ordered launcher presentation over exact host application entries.

use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::foundation::ColorRgba8;
use crate::input::{Activation, ChangeSource};
use crate::runtime::{RuntimeError, Ui};
use crate::shell::{
    ApplicationActionId, ApplicationEntry, ApplicationId, ApplicationRevision, ApplicationStates,
    InputSource, ShellCapabilities, ShellLayerKind, SystemRequest,
};
use crate::components::shell::primitives::{ShellLayerRef, ShellRootRef};
use crate::ui::{
    BoxStyle, ControlHandle, Flow, LayoutStyle, SemanticActions, SemanticCollection, SemanticName,
    SemanticNode, SemanticRole, SemanticState, UiNodeId,
};

/// Bounded caller order shared by launcher, panel, grid, and menu presentations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationCatalog(Arc<[ApplicationEntry]>);

impl ApplicationCatalog {
    pub const MAX_APPLICATIONS: usize = 4096;

    pub fn new(applications: Vec<ApplicationEntry>) -> Result<Self, ApplicationCatalogError> {
        if applications.len() > Self::MAX_APPLICATIONS {
            return Err(ApplicationCatalogError::TooMany {
                count: applications.len(),
                max: Self::MAX_APPLICATIONS,
            });
        }
        let mut seen = HashSet::with_capacity(applications.len());
        if let Some(application) = applications
            .iter()
            .map(ApplicationEntry::id)
            .find(|application| !seen.insert(*application))
        {
            return Err(ApplicationCatalogError::DuplicateApplication { application });
        }
        Ok(Self(applications.into()))
    }

    pub fn applications(&self) -> &[ApplicationEntry] {
        &self.0
    }

    pub fn application(&self, application: ApplicationId) -> Option<&ApplicationEntry> {
        self.0
            .iter()
            .find(|candidate| candidate.id() == application)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationCatalogError {
    TooMany { count: usize, max: usize },
    DuplicateApplication { application: ApplicationId },
}

impl fmt::Display for ApplicationCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid application catalog: {self:?}")
    }
}

impl std::error::Error for ApplicationCatalogError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationActionIntent {
    application: ApplicationId,
    revision: ApplicationRevision,
    action: ApplicationActionId,
    activation: Activation,
}

impl ApplicationActionIntent {
    pub(crate) const fn new(
        application: ApplicationId,
        revision: ApplicationRevision,
        action: ApplicationActionId,
        activation: Activation,
    ) -> Self {
        Self {
            application,
            revision,
            action,
            activation,
        }
    }

    pub const fn application(self) -> ApplicationId {
        self.application
    }

    pub const fn revision(self) -> ApplicationRevision {
        self.revision
    }

    pub const fn action(self) -> ApplicationActionId {
        self.action
    }

    pub const fn activation(self) -> Activation {
        self.activation
    }

    pub const fn inferred_source(self) -> Option<InputSource> {
        match self.activation.source {
            ChangeSource::Pointer => None,
            ChangeSource::Keyboard | ChangeSource::Directional => Some(InputSource::Keyboard),
            ChangeSource::Accessibility => Some(InputSource::Accessibility),
            ChangeSource::Programmatic => Some(InputSource::Programmatic),
        }
    }

    pub fn inferred_request(self) -> Option<SystemRequest> {
        self.inferred_source()
            .map(|source| self.build_request(source))
    }

    pub fn request(
        self,
        source: InputSource,
    ) -> Result<SystemRequest, ApplicationActionSourceError> {
        let matches = match self.activation.source {
            ChangeSource::Pointer => source.is_contact(),
            ChangeSource::Keyboard | ChangeSource::Directional => source == InputSource::Keyboard,
            ChangeSource::Accessibility => source == InputSource::Accessibility,
            ChangeSource::Programmatic => source == InputSource::Programmatic,
        };
        if !matches {
            return Err(ApplicationActionSourceError::SourceMismatch);
        }
        Ok(self.build_request(source))
    }

    const fn build_request(self, source: InputSource) -> SystemRequest {
        SystemRequest::ApplicationAction {
            application: self.application,
            revision: self.revision,
            action: self.action,
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationActionSourceError {
    SourceMismatch,
}

impl fmt::Display for ApplicationActionSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("application activation and shell input source do not match")
    }
}

impl std::error::Error for ApplicationActionSourceError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LauncherStyle {
    pub container: BoxStyle,
    pub item: BoxStyle,
    pub layout: LayoutStyle,
    pub label_color: ColorRgba8,
    pub label_size: f32,
}

impl Default for LauncherStyle {
    fn default() -> Self {
        Self {
            container: BoxStyle::default(),
            item: BoxStyle::default(),
            layout: LayoutStyle {
                flow: Flow::Vertical,
                ..LayoutStyle::default()
            },
            label_color: ColorRgba8::rgba(245, 247, 250, 255),
            label_size: 14.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Launcher {
    label: String,
    catalog: ApplicationCatalog,
    style: LauncherStyle,
}

impl Launcher {
    pub fn new(
        label: impl Into<String>,
        applications: Vec<ApplicationEntry>,
    ) -> Result<Self, LauncherError> {
        Self::from_catalog(label, ApplicationCatalog::new(applications)?)
    }

    pub fn from_catalog(
        label: impl Into<String>,
        catalog: ApplicationCatalog,
    ) -> Result<Self, LauncherError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(LauncherError::MissingAccessibleName);
        }
        Ok(Self {
            label,
            catalog,
            style: LauncherStyle::default(),
        })
    }

    pub fn style(mut self, style: LauncherStyle) -> Result<Self, LauncherError> {
        validate_label_size(style.label_size)?;
        self.style = style;
        Ok(self)
    }

    pub fn catalog(&self) -> &ApplicationCatalog {
        &self.catalog
    }

    pub fn mount<Action, Map>(
        &self,
        ui: &mut Ui<'_, '_, Action>,
        root: ShellRootRef,
        layer: ShellLayerRef,
        map: Map,
    ) -> Result<LauncherRef, LauncherMountError>
    where
        Action: 'static,
        Map: Fn(ApplicationActionIntent) -> Action + 'static,
    {
        validate_label_size(self.style.label_size)?;
        validate_launcher_layer(root, layer)?;
        let presentation = mount_application_catalog(
            ui,
            layer.content_node(),
            &self.label,
            &self.catalog,
            ApplicationPresentationStyle {
                container: self.style.container,
                item: self.style.item,
                layout: self.style.layout,
                label_color: self.style.label_color,
                label_size: self.style.label_size,
                container_role: SemanticRole::List,
                item_role: SemanticRole::Button,
            },
            root.grant()
                .permits(ShellCapabilities::INVOKE_SYSTEM_ACTION),
            map,
        )?;
        Ok(LauncherRef {
            container: presentation.container,
            catalog: self.catalog.clone(),
            items: presentation.items,
        })
    }
}

#[derive(Clone, Debug)]
pub struct LauncherRef {
    container: ControlHandle,
    catalog: ApplicationCatalog,
    items: Vec<ApplicationItemRef>,
}

impl LauncherRef {
    pub const fn node(&self) -> UiNodeId {
        self.container.node
    }

    pub fn catalog(&self) -> &ApplicationCatalog {
        &self.catalog
    }

    pub fn items(&self) -> &[ApplicationItemRef] {
        &self.items
    }

    pub fn item(&self, application: ApplicationId) -> Option<ApplicationItemRef> {
        self.items
            .iter()
            .copied()
            .find(|item| item.application == application)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ApplicationItemRef {
    control: ControlHandle,
    application: ApplicationId,
    revision: ApplicationRevision,
    action: Option<ApplicationActionId>,
    available: bool,
    states: ApplicationStates,
}

impl ApplicationItemRef {
    pub const fn node(self) -> UiNodeId {
        self.control.node
    }

    pub const fn application(self) -> ApplicationId {
        self.application
    }

    pub const fn revision(self) -> ApplicationRevision {
        self.revision
    }

    pub const fn action(self) -> Option<ApplicationActionId> {
        self.action
    }

    pub const fn available(self) -> bool {
        self.available
    }

    pub const fn states(self) -> ApplicationStates {
        self.states
    }
}

pub(crate) struct ApplicationPresentationRef {
    pub(crate) container: ControlHandle,
    pub(crate) items: Vec<ApplicationItemRef>,
}

#[derive(Clone, Copy)]
pub(crate) struct ApplicationPresentationStyle {
    pub(crate) container: BoxStyle,
    pub(crate) item: BoxStyle,
    pub(crate) layout: LayoutStyle,
    pub(crate) label_color: ColorRgba8,
    pub(crate) label_size: f32,
    pub(crate) container_role: SemanticRole,
    pub(crate) item_role: SemanticRole,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_application_catalog<Action, Map>(
    ui: &mut Ui<'_, '_, Action>,
    parent: UiNodeId,
    label: &str,
    catalog: &ApplicationCatalog,
    style: ApplicationPresentationStyle,
    authorized: bool,
    map: Map,
) -> Result<ApplicationPresentationRef, RuntimeError>
where
    Action: 'static,
    Map: Fn(ApplicationActionIntent) -> Action + 'static,
{
    let container = ui
        .foundation()
        .container_node_under(parent, style.container, style.layout, |_| {})
        .ok_or_else(|| RuntimeError::new("application presentation parent is stale"))?;
    let name = ui.foundation().intern(label);
    ui.foundation()
        .semantic_node(
            container.node,
            SemanticNode::named(style.container_role, name),
        )
        .map_err(application_semantic_error)?;

    let map = Rc::new(map);
    let item_count = u32::try_from(catalog.applications().len())
        .map_err(|_| RuntimeError::new("application catalog exceeds semantic collection bounds"))?;
    let mut items = Vec::with_capacity(catalog.applications().len());
    for (index, application) in catalog.applications().iter().enumerate() {
        let primary = application
            .primary_action()
            .and_then(|action| application.action(action));
        let available = authorized && primary.is_some_and(|action| action.enabled());
        let node = ui
            .foundation()
            .action_node_under(container.node, style.item, available, true, |writer| {
                writer.text(
                    application.label().as_str(),
                    style.label_color,
                    style.label_size,
                );
            })
            .ok_or_else(|| RuntimeError::new("application presentation container is stale"))?;
        let item_name = ui.foundation().intern(application.label().as_str());
        let description = application
            .description()
            .map(|description| ui.foundation().intern(description.as_str()));
        let actions = if available {
            SemanticActions::FOCUS | SemanticActions::ACTIVATE
        } else {
            SemanticActions::NONE
        };
        ui.foundation()
            .semantic_node(
                node.node,
                SemanticNode {
                    role: style.item_role,
                    name: SemanticName::Text(item_name),
                    description,
                    state: SemanticState {
                        disabled: !available,
                        focusable: available,
                        selected: Some(application.states().contains(ApplicationStates::ACTIVE)),
                        ..SemanticState::default()
                    },
                    actions,
                    collection: Some(SemanticCollection {
                        item_index: Some(u32::try_from(index).map_err(|_| {
                            RuntimeError::new("application index exceeds semantic bounds")
                        })?),
                        item_count: Some(item_count),
                        ..SemanticCollection::default()
                    }),
                    ..SemanticNode::default()
                },
            )
            .map_err(application_semantic_error)?;
        if available {
            let map = Rc::clone(&map);
            let application_id = application.id();
            let revision = application.revision();
            let action = primary
                .expect("available application has a primary action")
                .id();
            ui.route_activation(node.node, move |activation| {
                map(ApplicationActionIntent::new(
                    application_id,
                    revision,
                    action,
                    activation,
                ))
            })?;
        }
        items.push(ApplicationItemRef {
            control: node,
            application: application.id(),
            revision: application.revision(),
            action: primary.map(|action| action.id()),
            available,
            states: application.states(),
        });
    }
    Ok(ApplicationPresentationRef { container, items })
}

pub(crate) fn validate_launcher_layer(
    root: ShellRootRef,
    layer: ShellLayerRef,
) -> Result<(), LauncherError> {
    if layer.kind() != ShellLayerKind::Overlay {
        return Err(LauncherError::RequiresOverlayLayer);
    }
    if root.output() != layer.output() {
        return Err(LauncherError::OutputMismatch);
    }
    if root.grant().token() != layer.authority().grant() {
        return Err(LauncherError::GrantMismatch);
    }
    Ok(())
}

pub(crate) fn validate_label_size(label_size: f32) -> Result<(), LauncherError> {
    if !label_size.is_finite() || label_size <= 0.0 {
        return Err(LauncherError::InvalidLabelSize);
    }
    Ok(())
}

fn application_semantic_error(error: crate::ui::SemanticError) -> RuntimeError {
    RuntimeError::new(format!(
        "invalid application presentation semantics: {error:?}"
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LauncherError {
    Catalog(ApplicationCatalogError),
    MissingAccessibleName,
    InvalidLabelSize,
    RequiresOverlayLayer,
    OutputMismatch,
    GrantMismatch,
}

impl fmt::Display for LauncherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid launcher: {self:?}")
    }
}

impl std::error::Error for LauncherError {}

impl From<ApplicationCatalogError> for LauncherError {
    fn from(value: ApplicationCatalogError) -> Self {
        Self::Catalog(value)
    }
}

#[derive(Debug)]
pub enum LauncherMountError {
    Launcher(LauncherError),
    Runtime(RuntimeError),
}

impl fmt::Display for LauncherMountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launcher(error) => error.fmt(formatter),
            Self::Runtime(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LauncherMountError {}

impl From<LauncherError> for LauncherMountError {
    fn from(value: LauncherError) -> Self {
        Self::Launcher(value)
    }
}

impl From<RuntimeError> for LauncherMountError {
    fn from(value: RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

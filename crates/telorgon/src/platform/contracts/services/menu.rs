//! Platform-neutral native-menu publications and source-qualified action events.
//!
//! Menu trees are bounded immutable snapshots with stable item identities and exact revisions.
//! Accelerators reuse neutral physical shortcut chords while keeping their localized display text
//! separate and redacted. This module invokes no command or native API, retains no native menu
//! object, and owns no callback, queue, executor, thread, or event loop.

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU8, NonZeroU16, NonZeroU64};
use std::rc::Rc;
use std::sync::Arc;

use crate::input::{ShortcutChord, ShortcutTrigger};

use super::ServiceKey;
use crate::platform::contracts::{CapabilityDescriptor, RequestAdmission, Support, ViewId};

/// Maximum number of items in one complete neutral menu tree.
pub const MAX_MENU_ITEMS: usize = 1_024;
/// Maximum root-inclusive submenu depth in one complete neutral menu tree.
pub const MAX_MENU_DEPTH: u8 = 16;
/// Maximum number of accelerator-bearing items in one complete neutral menu tree.
pub const MAX_MENU_ACCELERATORS: usize = 512;
/// Maximum UTF-8 byte length of one menu item label.
pub const MAX_MENU_LABEL_BYTES: usize = 256;
/// Maximum UTF-8 byte length of one localized accelerator display label.
pub const MAX_MENU_ACCELERATOR_LABEL_BYTES: usize = 64;

/// Stable caller-owned identity for one item across menu revisions.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MenuItemId(NonZeroU64);

impl MenuItemId {
    pub const fn new(id: NonZeroU64) -> Self {
        Self(id)
    }

    pub const fn from_raw(id: u64) -> Option<Self> {
        match NonZeroU64::new(id) {
            Some(id) => Some(Self(id)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for MenuItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Stable host/application identity for one status-area or tray menu owner.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StatusMenuId(NonZeroU64);

impl StatusMenuId {
    pub const fn new(id: NonZeroU64) -> Self {
        Self(id)
    }

    pub const fn from_raw(id: u64) -> Option<Self> {
        match NonZeroU64::new(id) {
            Some(id) => Some(Self(id)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for StatusMenuId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Monotonic revision of one exact [`MenuScope`] history.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MenuRevision(NonZeroU64);

impl MenuRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(revision: NonZeroU64) -> Self {
        Self(revision)
    }

    pub const fn from_raw(revision: u64) -> Option<Self> {
        match NonZeroU64::new(revision) {
            Some(revision) => Some(Self(revision)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(revision) => Self::from_raw(revision),
            None => None,
        }
    }
}

impl fmt::Display for MenuRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Ownership scope of one independently revisioned native menu tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuScope {
    Application,
    View(ViewId),
    Status(StatusMenuId),
}

/// Exact identity of one complete menu snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuSnapshotId {
    scope: MenuScope,
    revision: MenuRevision,
}

impl MenuSnapshotId {
    pub const fn new(scope: MenuScope, revision: MenuRevision) -> Self {
        Self { scope, revision }
    }

    pub const fn scope(self) -> MenuScope {
        self.scope
    }

    pub const fn revision(self) -> MenuRevision {
        self.revision
    }
}

/// Bounded user-facing menu text whose contents are omitted from diagnostics.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MenuLabel(Arc<str>);

impl MenuLabel {
    pub fn new(value: impl AsRef<str>) -> Result<Self, MenuTextError> {
        validate_menu_text(value.as_ref(), MAX_MENU_LABEL_BYTES)?;
        Ok(Self(Arc::from(value.as_ref())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for MenuLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuLabel")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Bounded localized accelerator presentation, separate from the physical chord.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MenuAcceleratorLabel(Arc<str>);

impl MenuAcceleratorLabel {
    pub fn new(value: impl AsRef<str>) -> Result<Self, MenuTextError> {
        validate_menu_text(value.as_ref(), MAX_MENU_ACCELERATOR_LABEL_BYTES)?;
        Ok(Self(Arc::from(value.as_ref())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for MenuAcceleratorLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuAcceleratorLabel")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

fn validate_menu_text(value: &str, maximum_bytes: usize) -> Result<(), MenuTextError> {
    if value.trim().is_empty() {
        return Err(MenuTextError::Empty);
    }
    if value.len() > maximum_bytes {
        return Err(MenuTextError::TooLong {
            byte_len: value.len(),
            maximum_bytes,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(MenuTextError::ControlCharacter);
    }
    Ok(())
}

/// Invalid menu or accelerator display text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuTextError {
    Empty,
    TooLong {
        byte_len: usize,
        maximum_bytes: usize,
    },
    ControlCharacter,
}

impl fmt::Display for MenuTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("menu text is empty"),
            Self::TooLong {
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "menu text contains {byte_len} bytes; maximum is {maximum_bytes}"
            ),
            Self::ControlCharacter => formatter.write_str("menu text contains a control character"),
        }
    }
}

impl Error for MenuTextError {}

/// One exact accelerator matcher plus its caller-localized native presentation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuAccelerator {
    chord: ShortcutChord,
    display: MenuAcceleratorLabel,
}

impl MenuAccelerator {
    pub fn new(
        chord: ShortcutChord,
        display: MenuAcceleratorLabel,
    ) -> Result<Self, MenuAcceleratorError> {
        if chord.physical_key.get() == 0 {
            return Err(MenuAcceleratorError::UnknownPhysicalKey);
        }
        if chord.trigger != ShortcutTrigger::Pressed {
            return Err(MenuAcceleratorError::ReleasedTriggerUnsupported);
        }
        Ok(Self { chord, display })
    }

    pub const fn chord(&self) -> ShortcutChord {
        self.chord
    }

    pub const fn display(&self) -> &MenuAcceleratorLabel {
        &self.display
    }
}

/// Invalid native-menu accelerator declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuAcceleratorError {
    UnknownPhysicalKey,
    ReleasedTriggerUnsupported,
}

impl fmt::Display for MenuAcceleratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownPhysicalKey => "menu accelerator has an unknown physical key",
            Self::ReleasedTriggerUnsupported => {
                "native menu accelerators must use a pressed-key trigger"
            }
        })
    }
}

impl Error for MenuAcceleratorError {}

/// Platform-recognized semantic menu role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuRole {
    Application,
    File,
    Edit,
    View,
    Window,
    Help,
    Services,
    About,
    Settings,
    HideApplication,
    HideOtherApplications,
    ShowAllApplications,
    QuitApplication,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    MinimizeWindow,
    ZoomWindow,
    CloseWindow,
    BringAllToFront,
}

impl MenuRole {
    pub const fn item_kind(self) -> MenuItemKind {
        match self {
            Self::Application
            | Self::File
            | Self::Edit
            | Self::View
            | Self::Window
            | Self::Help
            | Self::Services => MenuItemKind::Submenu,
            Self::About
            | Self::Settings
            | Self::HideApplication
            | Self::HideOtherApplications
            | Self::ShowAllApplications
            | Self::QuitApplication
            | Self::Undo
            | Self::Redo
            | Self::Cut
            | Self::Copy
            | Self::Paste
            | Self::Delete
            | Self::SelectAll
            | Self::MinimizeWindow
            | Self::ZoomWindow
            | Self::CloseWindow
            | Self::BringAllToFront => MenuItemKind::Action,
        }
    }
}

/// Native menu item structural kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuItemKind {
    Action,
    Submenu,
    Separator,
}

/// Exact portable check-state presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuCheckState {
    #[default]
    NotCheckable,
    Unchecked,
    Checked,
    Mixed,
}

/// Caller-controlled menu item presentation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuItemState {
    enabled: bool,
    check: MenuCheckState,
    visible: bool,
}

impl MenuItemState {
    pub const fn new(enabled: bool, check: MenuCheckState, visible: bool) -> Self {
        Self {
            enabled,
            check,
            visible,
        }
    }

    pub const fn enabled(self) -> bool {
        self.enabled
    }

    pub const fn check(self) -> MenuCheckState {
        self.check
    }

    pub const fn visible(self) -> bool {
        self.visible
    }
}

impl Default for MenuItemState {
    fn default() -> Self {
        Self::new(true, MenuCheckState::NotCheckable, true)
    }
}

/// One immutable menu node. Constructors enforce kind-specific fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    id: MenuItemId,
    kind: MenuItemKind,
    label: Option<MenuLabel>,
    role: Option<MenuRole>,
    state: MenuItemState,
    accelerator: Option<MenuAccelerator>,
    children: Arc<[MenuItem]>,
}

impl MenuItem {
    pub fn action(
        id: MenuItemId,
        label: MenuLabel,
        role: Option<MenuRole>,
        state: MenuItemState,
        accelerator: Option<MenuAccelerator>,
    ) -> Result<Self, MenuItemError> {
        validate_role_kind(role, MenuItemKind::Action)?;
        Ok(Self {
            id,
            kind: MenuItemKind::Action,
            label: Some(label),
            role,
            state,
            accelerator,
            children: Arc::from([]),
        })
    }

    pub fn submenu(
        id: MenuItemId,
        label: MenuLabel,
        role: Option<MenuRole>,
        enabled: bool,
        visible: bool,
        children: Vec<MenuItem>,
    ) -> Result<Self, MenuItemError> {
        validate_role_kind(role, MenuItemKind::Submenu)?;
        Ok(Self {
            id,
            kind: MenuItemKind::Submenu,
            label: Some(label),
            role,
            state: MenuItemState::new(enabled, MenuCheckState::NotCheckable, visible),
            accelerator: None,
            children: children.into(),
        })
    }

    pub fn separator(id: MenuItemId) -> Self {
        Self {
            id,
            kind: MenuItemKind::Separator,
            label: None,
            role: None,
            state: MenuItemState::new(false, MenuCheckState::NotCheckable, true),
            accelerator: None,
            children: Arc::from([]),
        }
    }

    pub const fn id(&self) -> MenuItemId {
        self.id
    }

    pub const fn kind(&self) -> MenuItemKind {
        self.kind
    }

    pub const fn label(&self) -> Option<&MenuLabel> {
        self.label.as_ref()
    }

    pub const fn role(&self) -> Option<MenuRole> {
        self.role
    }

    pub const fn state(&self) -> MenuItemState {
        self.state
    }

    pub const fn accelerator(&self) -> Option<&MenuAccelerator> {
        self.accelerator.as_ref()
    }

    pub fn children(&self) -> &[MenuItem] {
        &self.children
    }
}

fn validate_role_kind(role: Option<MenuRole>, actual: MenuItemKind) -> Result<(), MenuItemError> {
    if let Some(role) = role
        && role.item_kind() != actual
    {
        return Err(MenuItemError::RoleKindMismatch {
            role,
            required: role.item_kind(),
            actual,
        });
    }
    Ok(())
}

/// Invalid kind-specific menu item declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuItemError {
    RoleKindMismatch {
        role: MenuRole,
        required: MenuItemKind,
        actual: MenuItemKind,
    },
}

impl fmt::Display for MenuItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RoleKindMismatch {
                role,
                required,
                actual,
            } => write!(
                formatter,
                "menu role {role:?} requires {required:?}, not {actual:?}"
            ),
        }
    }
}

impl Error for MenuItemError {}

/// Complete bounded immutable menu tree at one exact scope revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuTree {
    id: MenuSnapshotId,
    roots: Arc<[MenuItem]>,
    item_count: u16,
    depth: u8,
    accelerator_count: u16,
    has_native_roles: bool,
    has_mixed_check_state: bool,
}

impl MenuTree {
    pub fn new(
        scope: MenuScope,
        revision: MenuRevision,
        roots: Vec<MenuItem>,
    ) -> Result<Self, MenuTreeError> {
        let mut validation = MenuTreeValidation::default();
        validation.visit(&roots, 1, None)?;
        Ok(Self {
            id: MenuSnapshotId::new(scope, revision),
            roots: roots.into(),
            item_count: validation.item_ids.len() as u16,
            depth: validation.depth,
            accelerator_count: validation.accelerators.len() as u16,
            has_native_roles: !validation.roles.is_empty(),
            has_mixed_check_state: validation.has_mixed_check_state,
        })
    }

    pub const fn id(&self) -> MenuSnapshotId {
        self.id
    }

    pub const fn scope(&self) -> MenuScope {
        self.id.scope
    }

    pub const fn revision(&self) -> MenuRevision {
        self.id.revision
    }

    pub fn roots(&self) -> &[MenuItem] {
        &self.roots
    }

    pub const fn item_count(&self) -> u16 {
        self.item_count
    }

    pub const fn depth(&self) -> u8 {
        self.depth
    }

    pub const fn accelerator_count(&self) -> u16 {
        self.accelerator_count
    }

    pub const fn has_native_roles(&self) -> bool {
        self.has_native_roles
    }

    pub const fn has_mixed_check_state(&self) -> bool {
        self.has_mixed_check_state
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn item(&self, id: MenuItemId) -> Option<&MenuItem> {
        find_menu_item(&self.roots, id)
    }
}

fn find_menu_item(items: &[MenuItem], id: MenuItemId) -> Option<&MenuItem> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let Some(found) = find_menu_item(&item.children, id) {
            return Some(found);
        }
    }
    None
}

#[derive(Default)]
struct MenuTreeValidation {
    item_ids: Vec<MenuItemId>,
    roles: Vec<MenuRole>,
    accelerators: Vec<ShortcutChord>,
    depth: u8,
    has_mixed_check_state: bool,
}

impl MenuTreeValidation {
    fn visit(
        &mut self,
        items: &[MenuItem],
        depth: u8,
        parent: Option<MenuItemId>,
    ) -> Result<(), MenuTreeError> {
        if items.is_empty() {
            return Ok(());
        }
        if depth > MAX_MENU_DEPTH {
            return Err(MenuTreeError::TooDeep {
                supplied: depth,
                maximum: MAX_MENU_DEPTH,
            });
        }
        self.depth = self.depth.max(depth);
        if items[0].kind == MenuItemKind::Separator {
            return Err(MenuTreeError::LeadingSeparator {
                parent,
                separator: items[0].id,
            });
        }
        if items[items.len() - 1].kind == MenuItemKind::Separator {
            return Err(MenuTreeError::TrailingSeparator {
                parent,
                separator: items[items.len() - 1].id,
            });
        }

        for (index, item) in items.iter().enumerate() {
            if index > 0
                && item.kind == MenuItemKind::Separator
                && items[index - 1].kind == MenuItemKind::Separator
            {
                return Err(MenuTreeError::AdjacentSeparators {
                    parent,
                    first: items[index - 1].id,
                    second: item.id,
                });
            }
            if self.item_ids.contains(&item.id) {
                return Err(MenuTreeError::DuplicateItem { item: item.id });
            }
            self.item_ids.push(item.id);
            if self.item_ids.len() > MAX_MENU_ITEMS {
                return Err(MenuTreeError::TooManyItems {
                    supplied: self.item_ids.len(),
                    maximum: MAX_MENU_ITEMS,
                });
            }
            if let Some(role) = item.role {
                if self.roles.contains(&role) {
                    return Err(MenuTreeError::DuplicateRole { role });
                }
                self.roles.push(role);
            }
            if let Some(accelerator) = &item.accelerator {
                let chord = accelerator.chord();
                if self.accelerators.contains(&chord) {
                    return Err(MenuTreeError::DuplicateAccelerator { chord });
                }
                self.accelerators.push(chord);
                if self.accelerators.len() > MAX_MENU_ACCELERATORS {
                    return Err(MenuTreeError::TooManyAccelerators {
                        supplied: self.accelerators.len(),
                        maximum: MAX_MENU_ACCELERATORS,
                    });
                }
            }
            self.has_mixed_check_state |= item.state.check == MenuCheckState::Mixed;
            if item.kind == MenuItemKind::Submenu {
                self.visit(&item.children, depth + 1, Some(item.id))?;
            }
        }
        Ok(())
    }
}

/// Invalid complete menu-tree topology or metadata relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuTreeError {
    TooManyItems {
        supplied: usize,
        maximum: usize,
    },
    TooDeep {
        supplied: u8,
        maximum: u8,
    },
    TooManyAccelerators {
        supplied: usize,
        maximum: usize,
    },
    DuplicateItem {
        item: MenuItemId,
    },
    DuplicateRole {
        role: MenuRole,
    },
    DuplicateAccelerator {
        chord: ShortcutChord,
    },
    LeadingSeparator {
        parent: Option<MenuItemId>,
        separator: MenuItemId,
    },
    TrailingSeparator {
        parent: Option<MenuItemId>,
        separator: MenuItemId,
    },
    AdjacentSeparators {
        parent: Option<MenuItemId>,
        first: MenuItemId,
        second: MenuItemId,
    },
}

impl fmt::Display for MenuTreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyItems { .. } => "menu tree exceeds the item-count bound",
            Self::TooDeep { .. } => "menu tree exceeds the depth bound",
            Self::TooManyAccelerators { .. } => "menu tree exceeds the accelerator-count bound",
            Self::DuplicateItem { .. } => "menu tree repeats an item identity",
            Self::DuplicateRole { .. } => "menu tree repeats a native role",
            Self::DuplicateAccelerator { .. } => "menu tree repeats an accelerator chord",
            Self::LeadingSeparator { .. } => "menu sibling list begins with a separator",
            Self::TrailingSeparator { .. } => "menu sibling list ends with a separator",
            Self::AdjacentSeparators { .. } => "menu sibling list has adjacent separators",
        })
    }
}

impl Error for MenuTreeError {}

/// Exact initial or advancing publication of one complete menu tree.
#[derive(Clone, PartialEq, Eq)]
pub struct MenuPublicationRequest {
    previous: Option<MenuSnapshotId>,
    tree: MenuTree,
}

impl MenuPublicationRequest {
    pub fn initial(tree: MenuTree) -> Result<Self, MenuPublicationError> {
        if tree.revision() != MenuRevision::INITIAL {
            return Err(MenuPublicationError::InitialRevisionRequired {
                supplied: tree.revision(),
            });
        }
        Ok(Self {
            previous: None,
            tree,
        })
    }

    pub fn advance(previous: MenuSnapshotId, tree: MenuTree) -> Result<Self, MenuPublicationError> {
        if previous.scope() != tree.scope() {
            return Err(MenuPublicationError::ScopeMismatch {
                previous: previous.scope(),
                current: tree.scope(),
            });
        }
        let Some(expected) = previous.revision().checked_next() else {
            return Err(MenuPublicationError::RevisionExhausted);
        };
        if tree.revision() != expected {
            return Err(MenuPublicationError::RevisionNotNext {
                previous: previous.revision(),
                current: tree.revision(),
            });
        }
        Ok(Self {
            previous: Some(previous),
            tree,
        })
    }

    pub const fn previous(&self) -> Option<MenuSnapshotId> {
        self.previous
    }

    pub const fn tree(&self) -> &MenuTree {
        &self.tree
    }

    pub fn into_tree(self) -> MenuTree {
        self.tree
    }
}

impl fmt::Debug for MenuPublicationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MenuPublicationRequest")
            .field("previous", &self.previous)
            .field("current", &self.tree.id())
            .field("item_count", &self.tree.item_count())
            .field("depth", &self.tree.depth())
            .finish_non_exhaustive()
    }
}

/// Invalid revision relationship in a menu publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuPublicationError {
    InitialRevisionRequired {
        supplied: MenuRevision,
    },
    ScopeMismatch {
        previous: MenuScope,
        current: MenuScope,
    },
    RevisionNotNext {
        previous: MenuRevision,
        current: MenuRevision,
    },
    RevisionExhausted,
}

impl fmt::Display for MenuPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InitialRevisionRequired { .. } => {
                "initial menu publication must use the initial revision"
            }
            Self::ScopeMismatch { .. } => "menu publication changes revision scope",
            Self::RevisionNotNext { .. } => "menu publication revision is not the exact successor",
            Self::RevisionExhausted => "menu publication revision is exhausted",
        })
    }
}

impl Error for MenuPublicationError {}

/// Metadata returned after an exact menu publication applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuPublicationApplied {
    snapshot: MenuSnapshotId,
    item_count: u16,
}

impl MenuPublicationApplied {
    pub const fn from_request(request: &MenuPublicationRequest) -> Self {
        Self {
            snapshot: request.tree.id,
            item_count: request.tree.item_count,
        }
    }

    pub const fn snapshot(self) -> MenuSnapshotId {
        self.snapshot
    }

    pub const fn item_count(self) -> u16 {
        self.item_count
    }
}

/// Independently discoverable native-menu features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MenuOperations {
    application_menus: bool,
    view_menus: bool,
    status_menus: bool,
    native_roles: bool,
    accelerators: bool,
    mixed_check_state: bool,
    action_events: bool,
}

impl MenuOperations {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        application_menus: bool,
        view_menus: bool,
        status_menus: bool,
        native_roles: bool,
        accelerators: bool,
        mixed_check_state: bool,
        action_events: bool,
    ) -> Self {
        Self {
            application_menus,
            view_menus,
            status_menus,
            native_roles,
            accelerators,
            mixed_check_state,
            action_events,
        }
    }

    pub const fn supports_scope(self, scope: MenuScope) -> bool {
        match scope {
            MenuScope::Application => self.application_menus,
            MenuScope::View(_) => self.view_menus,
            MenuScope::Status(_) => self.status_menus,
        }
    }

    pub const fn supports_native_roles(self) -> bool {
        self.native_roles
    }

    pub const fn supports_accelerators(self) -> bool {
        self.accelerators
    }

    pub const fn supports_mixed_check_state(self) -> bool {
        self.mixed_check_state
    }

    pub const fn supports_action_events(self) -> bool {
        self.action_events
    }
}

/// Adapter-advertised tree bounds capped by neutral hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuLimits {
    maximum_items: NonZeroU16,
    maximum_depth: NonZeroU8,
    maximum_accelerators: NonZeroU16,
}

impl MenuLimits {
    pub const fn new(
        maximum_items: NonZeroU16,
        maximum_depth: NonZeroU8,
        maximum_accelerators: NonZeroU16,
    ) -> Result<Self, MenuLimitError> {
        if maximum_items.get() as usize > MAX_MENU_ITEMS {
            return Err(MenuLimitError::ItemLimitTooLarge);
        }
        if maximum_depth.get() > MAX_MENU_DEPTH {
            return Err(MenuLimitError::DepthLimitTooLarge);
        }
        if maximum_accelerators.get() as usize > MAX_MENU_ACCELERATORS {
            return Err(MenuLimitError::AcceleratorLimitTooLarge);
        }
        if maximum_accelerators.get() > maximum_items.get() {
            return Err(MenuLimitError::AcceleratorsExceedItems);
        }
        Ok(Self {
            maximum_items,
            maximum_depth,
            maximum_accelerators,
        })
    }

    pub const fn maximum_items(self) -> NonZeroU16 {
        self.maximum_items
    }

    pub const fn maximum_depth(self) -> NonZeroU8 {
        self.maximum_depth
    }

    pub const fn maximum_accelerators(self) -> NonZeroU16 {
        self.maximum_accelerators
    }
}

impl Default for MenuLimits {
    fn default() -> Self {
        Self {
            maximum_items: NonZeroU16::new(MAX_MENU_ITEMS as u16)
                .expect("menu item hard bound is nonzero"),
            maximum_depth: NonZeroU8::new(MAX_MENU_DEPTH)
                .expect("menu depth hard bound is nonzero"),
            maximum_accelerators: NonZeroU16::new(MAX_MENU_ACCELERATORS as u16)
                .expect("menu accelerator hard bound is nonzero"),
        }
    }
}

/// Invalid host-advertised menu limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuLimitError {
    ItemLimitTooLarge,
    DepthLimitTooLarge,
    AcceleratorLimitTooLarge,
    AcceleratorsExceedItems,
}

impl fmt::Display for MenuLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ItemLimitTooLarge => "menu item limit exceeds the neutral hard bound",
            Self::DepthLimitTooLarge => "menu depth limit exceeds the neutral hard bound",
            Self::AcceleratorLimitTooLarge => {
                "menu accelerator limit exceeds the neutral hard bound"
            }
            Self::AcceleratorsExceedItems => "menu accelerator limit exceeds the item limit",
        })
    }
}

impl Error for MenuLimitError {}

pub type MenuCapability = CapabilityDescriptor<MenuOperations, MenuLimits>;

/// Exact menu scope for capability discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuCapabilityQuery {
    scope: MenuScope,
}

impl MenuCapabilityQuery {
    pub const fn new(scope: MenuScope) -> Self {
        Self { scope }
    }

    pub const fn scope(self) -> MenuScope {
        self.scope
    }
}

/// Immediate rejection before a menu publication is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuAdmissionError {
    ScopeUnavailable {
        scope: MenuScope,
    },
    UnsupportedScope {
        scope: MenuScope,
    },
    PermissionDenied,
    NativeRolesUnsupported,
    AcceleratorsUnsupported,
    MixedCheckStateUnsupported,
    ItemsExceedCapability,
    DepthExceedsCapability,
    AcceleratorsExceedCapability,
    RevisionMismatch {
        expected_previous: Option<MenuRevision>,
        observed: Option<MenuRevision>,
    },
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for MenuAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ScopeUnavailable { .. } => "menu scope is unavailable",
            Self::UnsupportedScope { .. } => "menu scope is unsupported",
            Self::PermissionDenied => "menu publication permission is denied",
            Self::NativeRolesUnsupported => "native menu roles are unsupported",
            Self::AcceleratorsUnsupported => "native menu accelerators are unsupported",
            Self::MixedCheckStateUnsupported => "mixed menu check state is unsupported",
            Self::ItemsExceedCapability => "menu item count exceeds capability",
            Self::DepthExceedsCapability => "menu depth exceeds capability",
            Self::AcceleratorsExceedCapability => "menu accelerator count exceeds capability",
            Self::RevisionMismatch { .. } => "menu publication cites a stale previous revision",
            Self::CapabilityChanged => "menu capability changed before admission",
            Self::CapacityExceeded => "menu publication admission capacity was exceeded",
        })
    }
}

impl Error for MenuAdmissionError {}

pub type MenuPublicationAdmission = RequestAdmission<MenuPublicationApplied, MenuAdmissionError>;

/// Source of one native menu activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuActionSource {
    Pointer,
    KeyboardNavigation,
    Accelerator,
    AssistiveTechnology,
    PlatformRole,
    StatusItem,
}

/// Adapter-observed action candidate citing an exact menu snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuActionRequest {
    snapshot: MenuSnapshotId,
    item: MenuItemId,
    source: MenuActionSource,
}

impl MenuActionRequest {
    pub const fn new(snapshot: MenuSnapshotId, item: MenuItemId, source: MenuActionSource) -> Self {
        Self {
            snapshot,
            item,
            source,
        }
    }

    pub const fn snapshot(self) -> MenuSnapshotId {
        self.snapshot
    }

    pub const fn item(self) -> MenuItemId {
        self.item
    }

    pub const fn source(self) -> MenuActionSource {
        self.source
    }
}

/// Validated source-qualified portable menu action event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MenuActionEvent {
    snapshot: MenuSnapshotId,
    item: MenuItemId,
    role: Option<MenuRole>,
    source: MenuActionSource,
}

impl MenuActionEvent {
    pub fn admit(current: &MenuTree, request: MenuActionRequest) -> MenuActionAdmission {
        if request.snapshot.scope != current.scope() {
            return Err(MenuActionAdmissionError::ScopeMismatch {
                expected: current.scope(),
                observed: request.snapshot.scope,
            });
        }
        if request.snapshot.revision != current.revision() {
            return Err(MenuActionAdmissionError::StaleRevision {
                expected: current.revision(),
                observed: request.snapshot.revision,
            });
        }
        let Some(item) = current.item(request.item) else {
            return Err(MenuActionAdmissionError::UnknownItem { item: request.item });
        };
        if item.kind != MenuItemKind::Action {
            return Err(MenuActionAdmissionError::ItemNotActionable { item: request.item });
        }
        if !item.state.visible {
            return Err(MenuActionAdmissionError::ItemHidden { item: request.item });
        }
        if !item.state.enabled {
            return Err(MenuActionAdmissionError::ItemDisabled { item: request.item });
        }
        if request.source == MenuActionSource::Accelerator && item.accelerator.is_none() {
            return Err(MenuActionAdmissionError::AcceleratorNotAdvertised { item: request.item });
        }
        if request.source == MenuActionSource::PlatformRole && item.role.is_none() {
            return Err(MenuActionAdmissionError::RoleNotAdvertised { item: request.item });
        }
        if request.source == MenuActionSource::StatusItem
            && !matches!(current.scope(), MenuScope::Status(_))
        {
            return Err(MenuActionAdmissionError::StatusSourceOutsideStatusScope);
        }
        Ok(Self {
            snapshot: request.snapshot,
            item: request.item,
            role: item.role,
            source: request.source,
        })
    }

    pub const fn snapshot(self) -> MenuSnapshotId {
        self.snapshot
    }

    pub const fn item(self) -> MenuItemId {
        self.item
    }

    pub const fn role(self) -> Option<MenuRole> {
        self.role
    }

    pub const fn source(self) -> MenuActionSource {
        self.source
    }
}

/// Rejection before a native menu activation becomes a portable event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuActionAdmissionError {
    ScopeMismatch {
        expected: MenuScope,
        observed: MenuScope,
    },
    StaleRevision {
        expected: MenuRevision,
        observed: MenuRevision,
    },
    UnknownItem {
        item: MenuItemId,
    },
    ItemNotActionable {
        item: MenuItemId,
    },
    ItemHidden {
        item: MenuItemId,
    },
    ItemDisabled {
        item: MenuItemId,
    },
    AcceleratorNotAdvertised {
        item: MenuItemId,
    },
    RoleNotAdvertised {
        item: MenuItemId,
    },
    StatusSourceOutsideStatusScope,
}

impl fmt::Display for MenuActionAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ScopeMismatch { .. } => "menu action cites another menu scope",
            Self::StaleRevision { .. } => "menu action cites a stale menu revision",
            Self::UnknownItem { .. } => "menu action cites an unknown item",
            Self::ItemNotActionable { .. } => "menu action cites a submenu or separator",
            Self::ItemHidden { .. } => "menu action cites a hidden item",
            Self::ItemDisabled { .. } => "menu action cites a disabled item",
            Self::AcceleratorNotAdvertised { .. } => {
                "accelerator action cites an item without an accelerator"
            }
            Self::RoleNotAdvertised { .. } => {
                "platform-role action cites an item without a native role"
            }
            Self::StatusSourceOutsideStatusScope => {
                "status-item action cites a non-status menu scope"
            }
        })
    }
}

impl Error for MenuActionAdmissionError {}

pub type MenuActionAdmission = Result<MenuActionEvent, MenuActionAdmissionError>;

/// Object-safe capability and complete-tree publication boundary.
pub trait MenuService {
    fn capability(&self, query: MenuCapabilityQuery) -> Support<MenuCapability>;

    fn publish(&self, request: MenuPublicationRequest) -> MenuPublicationAdmission;
}

pub enum MenuServiceKey {}

impl ServiceKey for MenuServiceKey {
    type Handle = Rc<dyn MenuService>;
}

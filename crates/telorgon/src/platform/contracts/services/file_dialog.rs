//! Platform-neutral asynchronous file-dialog intentions and selected resources.
//!
//! Results are redacted [`ExternalUri`] locators, not assumed filesystem paths. Optional sandbox
//! grants remain opaque, non-cloneable adapter values. This module opens no dialog, performs no
//! file I/O, retains no native dialog object, and owns no callback, queue, executor, thread, or
//! event loop.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::rc::Rc;
use std::sync::Arc;

use super::ServiceKey;
use super::data_transfer::DataFormat;
use super::uri::ExternalUri;
use crate::platform::contracts::{
    CapabilityDescriptor, RequestAdmission, Support, UserGestureGrantHandle, ViewId,
};

pub const MAX_FILE_DIALOG_FILTERS: usize = 16;
pub const MAX_FILE_DIALOG_FILTER_RULES: usize = 16;
pub const MAX_FILE_DIALOG_FILTER_LABEL_BYTES: usize = 256;
pub const MAX_FILE_EXTENSION_BYTES: usize = 32;
pub const MAX_SUGGESTED_FILE_NAME_BYTES: usize = 255;
pub const MAX_SELECTED_RESOURCES: usize = 64;
pub const MAX_SELECTED_RESOURCE_NAME_BYTES: usize = 255;

/// Native dialog family requested by portable code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogMode {
    OpenFile,
    SaveFile,
    SelectFolder,
}

/// Normalized file-extension filter without a leading dot.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileExtension(Arc<str>);

impl FileExtension {
    pub fn new(value: impl AsRef<str>) -> Result<Self, FileExtensionError> {
        let value = value.as_ref();
        let value = value.strip_prefix('.').unwrap_or(value);
        if value.is_empty() {
            return Err(FileExtensionError::Empty);
        }
        if value.len() > MAX_FILE_EXTENSION_BYTES {
            return Err(FileExtensionError::TooLong);
        }
        if !value.is_ascii() {
            return Err(FileExtensionError::NonAscii);
        }
        let bytes = value.as_bytes();
        if !bytes[0].is_ascii_alphanumeric()
            || !bytes[bytes.len() - 1].is_ascii_alphanumeric()
            || bytes.iter().any(|byte| {
                !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'-' | b'.' | b'_')
            })
        {
            return Err(FileExtensionError::InvalidCharacter);
        }
        Ok(Self(Arc::from(value.to_ascii_lowercase())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FileExtension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FileExtension")
            .field(&self.as_str())
            .finish()
    }
}

/// Invalid extension filter metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileExtensionError {
    Empty,
    TooLong,
    NonAscii,
    InvalidCharacter,
}

impl fmt::Display for FileExtensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "file extension is empty",
            Self::TooLong => "file extension exceeds the metadata bound",
            Self::NonAscii => "file extension must be ASCII",
            Self::InvalidCharacter => "file extension contains an invalid character",
        })
    }
}

impl Error for FileExtensionError {}

/// One exact typed file-dialog filter rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogFilterRule {
    Extension(FileExtension),
    Format(DataFormat),
}

/// Bounded native-dialog filter with a redacted user-facing label.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileDialogFilter {
    label: Arc<str>,
    rules: Arc<[FileDialogFilterRule]>,
}

impl FileDialogFilter {
    pub fn new(
        label: impl AsRef<str>,
        rules: Vec<FileDialogFilterRule>,
    ) -> Result<Self, FileDialogFilterError> {
        let label = label.as_ref();
        if label.trim().is_empty() {
            return Err(FileDialogFilterError::EmptyLabel);
        }
        if label.len() > MAX_FILE_DIALOG_FILTER_LABEL_BYTES {
            return Err(FileDialogFilterError::LabelTooLong);
        }
        if label.chars().any(char::is_control) {
            return Err(FileDialogFilterError::ControlCharacterInLabel);
        }
        if rules.is_empty() {
            return Err(FileDialogFilterError::EmptyRules);
        }
        if rules.len() > MAX_FILE_DIALOG_FILTER_RULES {
            return Err(FileDialogFilterError::TooManyRules);
        }
        for (index, rule) in rules.iter().enumerate() {
            if rules[..index].contains(rule) {
                return Err(FileDialogFilterError::DuplicateRule);
            }
        }
        Ok(Self {
            label: Arc::from(label),
            rules: rules.into(),
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn rules(&self) -> &[FileDialogFilterRule] {
        &self.rules
    }
}

impl fmt::Debug for FileDialogFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileDialogFilter")
            .field("label_byte_len", &self.label.len())
            .field("rule_count", &self.rules.len())
            .field("label_redacted", &true)
            .finish_non_exhaustive()
    }
}

/// Invalid file-dialog filter metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogFilterError {
    EmptyLabel,
    LabelTooLong,
    ControlCharacterInLabel,
    EmptyRules,
    TooManyRules,
    DuplicateRule,
}

impl fmt::Display for FileDialogFilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLabel => "file-dialog filter label is empty",
            Self::LabelTooLong => "file-dialog filter label exceeds the metadata bound",
            Self::ControlCharacterInLabel => {
                "file-dialog filter label contains a control character"
            }
            Self::EmptyRules => "file-dialog filter contains no rules",
            Self::TooManyRules => "file-dialog filter exceeds the rule-count bound",
            Self::DuplicateRule => "file-dialog filter contains a duplicate rule",
        })
    }
}

impl Error for FileDialogFilterError {}

/// Redacted filename suggestion for a save dialog.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SuggestedFileName(Arc<str>);

impl SuggestedFileName {
    pub fn new(value: impl AsRef<str>) -> Result<Self, SuggestedFileNameError> {
        let value = value.as_ref();
        if value.trim().is_empty() {
            return Err(SuggestedFileNameError::Empty);
        }
        if value.len() > MAX_SUGGESTED_FILE_NAME_BYTES {
            return Err(SuggestedFileNameError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(SuggestedFileNameError::ControlCharacter);
        }
        if value.contains('/') || value.contains('\\') || matches!(value, "." | "..") {
            return Err(SuggestedFileNameError::PathLike);
        }
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for SuggestedFileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SuggestedFileName")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Invalid save-dialog filename suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SuggestedFileNameError {
    Empty,
    TooLong,
    ControlCharacter,
    PathLike,
}

impl fmt::Display for SuggestedFileNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "suggested file name is empty",
            Self::TooLong => "suggested file name exceeds the metadata bound",
            Self::ControlCharacter => "suggested file name contains a control character",
            Self::PathLike => "suggested file name must not contain a path",
        })
    }
}

impl Error for SuggestedFileNameError {}

/// Whether selected resources must carry adapter-owned sandbox access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SandboxAccessPolicy {
    #[default]
    PlatformDefault,
    RequireGrant,
}

/// Validated immutable dialog options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDialogOptions {
    mode: FileDialogMode,
    filters: Arc<[FileDialogFilter]>,
    suggested_name: Option<SuggestedFileName>,
    selection_limit: NonZeroU16,
    sandbox_access: SandboxAccessPolicy,
}

impl FileDialogOptions {
    pub fn new(
        mode: FileDialogMode,
        filters: Vec<FileDialogFilter>,
        suggested_name: Option<SuggestedFileName>,
        selection_limit: NonZeroU16,
        sandbox_access: SandboxAccessPolicy,
    ) -> Result<Self, FileDialogOptionsError> {
        if filters.len() > MAX_FILE_DIALOG_FILTERS {
            return Err(FileDialogOptionsError::TooManyFilters);
        }
        for (index, filter) in filters.iter().enumerate() {
            if filters[..index].contains(filter) {
                return Err(FileDialogOptionsError::DuplicateFilter);
            }
        }
        if selection_limit.get() as usize > MAX_SELECTED_RESOURCES {
            return Err(FileDialogOptionsError::SelectionLimitTooLarge);
        }
        if mode == FileDialogMode::SaveFile && selection_limit.get() != 1 {
            return Err(FileDialogOptionsError::SaveRequiresSingleSelection);
        }
        if mode != FileDialogMode::SaveFile && suggested_name.is_some() {
            return Err(FileDialogOptionsError::SuggestedNameRequiresSave);
        }
        Ok(Self {
            mode,
            filters: filters.into(),
            suggested_name,
            selection_limit,
            sandbox_access,
        })
    }

    pub const fn mode(&self) -> FileDialogMode {
        self.mode
    }

    pub fn filters(&self) -> &[FileDialogFilter] {
        &self.filters
    }

    pub const fn suggested_name(&self) -> Option<&SuggestedFileName> {
        self.suggested_name.as_ref()
    }

    pub const fn selection_limit(&self) -> NonZeroU16 {
        self.selection_limit
    }

    pub const fn allows_multiple(&self) -> bool {
        self.selection_limit.get() > 1
    }

    pub const fn sandbox_access(&self) -> SandboxAccessPolicy {
        self.sandbox_access
    }
}

/// Invalid cross-field file-dialog options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogOptionsError {
    TooManyFilters,
    DuplicateFilter,
    SelectionLimitTooLarge,
    SaveRequiresSingleSelection,
    SuggestedNameRequiresSave,
}

impl fmt::Display for FileDialogOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyFilters => "file dialog exceeds the filter-count bound",
            Self::DuplicateFilter => "file dialog contains a duplicate filter",
            Self::SelectionLimitTooLarge => "file dialog selection limit exceeds the hard bound",
            Self::SaveRequiresSingleSelection => "save dialog requires exactly one selection",
            Self::SuggestedNameRequiresSave => "suggested file name requires save mode",
        })
    }
}

impl Error for FileDialogOptionsError {}

/// Independently discoverable file-dialog operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileDialogOperations {
    open_file: bool,
    save_file: bool,
    select_folder: bool,
    multiple_selection: bool,
    sandbox_grants: bool,
}

impl FileDialogOperations {
    pub const fn new(
        open_file: bool,
        save_file: bool,
        select_folder: bool,
        multiple_selection: bool,
        sandbox_grants: bool,
    ) -> Self {
        Self {
            open_file,
            save_file,
            select_folder,
            multiple_selection,
            sandbox_grants,
        }
    }

    pub const fn supports_mode(self, mode: FileDialogMode) -> bool {
        match mode {
            FileDialogMode::OpenFile => self.open_file,
            FileDialogMode::SaveFile => self.save_file,
            FileDialogMode::SelectFolder => self.select_folder,
        }
    }

    pub const fn supports_multiple_selection(self) -> bool {
        self.multiple_selection
    }

    pub const fn supports_sandbox_grants(self) -> bool {
        self.sandbox_grants
    }
}

/// Adapter-advertised bounds capped by the neutral hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileDialogLimits {
    maximum_filters: NonZeroU16,
    maximum_rules_per_filter: NonZeroU16,
    maximum_selections: NonZeroU16,
    maximum_suggested_name_bytes: NonZeroU32,
}

impl FileDialogLimits {
    pub const fn new(
        maximum_filters: NonZeroU16,
        maximum_rules_per_filter: NonZeroU16,
        maximum_selections: NonZeroU16,
        maximum_suggested_name_bytes: NonZeroU32,
    ) -> Result<Self, FileDialogLimitError> {
        if maximum_filters.get() as usize > MAX_FILE_DIALOG_FILTERS {
            return Err(FileDialogLimitError::FilterLimitTooLarge);
        }
        if maximum_rules_per_filter.get() as usize > MAX_FILE_DIALOG_FILTER_RULES {
            return Err(FileDialogLimitError::RuleLimitTooLarge);
        }
        if maximum_selections.get() as usize > MAX_SELECTED_RESOURCES {
            return Err(FileDialogLimitError::SelectionLimitTooLarge);
        }
        if maximum_suggested_name_bytes.get() as usize > MAX_SUGGESTED_FILE_NAME_BYTES {
            return Err(FileDialogLimitError::SuggestedNameLimitTooLarge);
        }
        Ok(Self {
            maximum_filters,
            maximum_rules_per_filter,
            maximum_selections,
            maximum_suggested_name_bytes,
        })
    }

    pub const fn maximum_filters(self) -> NonZeroU16 {
        self.maximum_filters
    }

    pub const fn maximum_rules_per_filter(self) -> NonZeroU16 {
        self.maximum_rules_per_filter
    }

    pub const fn maximum_selections(self) -> NonZeroU16 {
        self.maximum_selections
    }

    pub const fn maximum_suggested_name_bytes(self) -> NonZeroU32 {
        self.maximum_suggested_name_bytes
    }
}

impl Default for FileDialogLimits {
    fn default() -> Self {
        Self {
            maximum_filters: NonZeroU16::new(MAX_FILE_DIALOG_FILTERS as u16)
                .expect("the hard filter limit is nonzero"),
            maximum_rules_per_filter: NonZeroU16::new(MAX_FILE_DIALOG_FILTER_RULES as u16)
                .expect("the hard rule limit is nonzero"),
            maximum_selections: NonZeroU16::new(MAX_SELECTED_RESOURCES as u16)
                .expect("the hard selection limit is nonzero"),
            maximum_suggested_name_bytes: NonZeroU32::new(MAX_SUGGESTED_FILE_NAME_BYTES as u32)
                .expect("the hard suggested-name limit is nonzero"),
        }
    }
}

/// Invalid host-advertised dialog limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogLimitError {
    FilterLimitTooLarge,
    RuleLimitTooLarge,
    SelectionLimitTooLarge,
    SuggestedNameLimitTooLarge,
}

impl fmt::Display for FileDialogLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FilterLimitTooLarge => "file-dialog filter limit exceeds the hard bound",
            Self::RuleLimitTooLarge => "file-dialog rule limit exceeds the hard bound",
            Self::SelectionLimitTooLarge => "file-dialog selection limit exceeds the hard bound",
            Self::SuggestedNameLimitTooLarge => {
                "file-dialog suggested-name limit exceeds the hard bound"
            }
        })
    }
}

impl Error for FileDialogLimitError {}

pub type FileDialogCapability = CapabilityDescriptor<FileDialogOperations, FileDialogLimits>;

/// Per-view file-dialog capability scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileDialogCapabilityQuery {
    view: ViewId,
}

impl FileDialogCapabilityQuery {
    pub const fn new(view: ViewId) -> Self {
        Self { view }
    }

    pub const fn view(self) -> ViewId {
        self.view
    }
}

/// Opaque adapter-owned sandbox or security-scoped access grant.
///
/// Concrete implementations may release temporary access from `Drop`. Portable code cannot clone,
/// inspect, compare, or debug-format the native token.
pub trait SandboxAccessGrant: 'static {
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

pub type SandboxAccessGrantHandle = Box<dyn SandboxAccessGrant>;

/// Kind of selected resource without assuming its locator is a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectedResourceKind {
    File,
    Folder,
}

/// Access intent granted for a selected resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectedResourceAccess {
    Read,
    Write,
    ReadWrite,
}

impl SelectedResourceAccess {
    pub const fn supports_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    pub const fn supports_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

/// Optional bounded display name for a selected resource.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SelectedResourceName(Arc<str>);

impl SelectedResourceName {
    pub fn new(value: impl AsRef<str>) -> Result<Self, SelectedResourceNameError> {
        let value = value.as_ref();
        if value.trim().is_empty() {
            return Err(SelectedResourceNameError::Empty);
        }
        if value.len() > MAX_SELECTED_RESOURCE_NAME_BYTES {
            return Err(SelectedResourceNameError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(SelectedResourceNameError::ControlCharacter);
        }
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for SelectedResourceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedResourceName")
            .field("byte_len", &self.byte_len())
            .field("redacted", &true)
            .finish()
    }
}

/// Invalid resource display-name metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectedResourceNameError {
    Empty,
    TooLong,
    ControlCharacter,
}

impl fmt::Display for SelectedResourceNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "selected resource name is empty",
            Self::TooLong => "selected resource name exceeds the metadata bound",
            Self::ControlCharacter => "selected resource name contains a control character",
        })
    }
}

impl Error for SelectedResourceNameError {}

/// One selected resource with redacted locator/name and optional opaque access grant.
pub struct SelectedResource {
    kind: SelectedResourceKind,
    locator: ExternalUri,
    display_name: Option<SelectedResourceName>,
    access: SelectedResourceAccess,
    sandbox_grant: Option<SandboxAccessGrantHandle>,
}

impl SelectedResource {
    pub fn new(
        kind: SelectedResourceKind,
        locator: ExternalUri,
        display_name: Option<SelectedResourceName>,
        access: SelectedResourceAccess,
        sandbox_grant: Option<SandboxAccessGrantHandle>,
    ) -> Self {
        Self {
            kind,
            locator,
            display_name,
            access,
            sandbox_grant,
        }
    }

    pub const fn kind(&self) -> SelectedResourceKind {
        self.kind
    }

    pub const fn locator(&self) -> &ExternalUri {
        &self.locator
    }

    pub const fn display_name(&self) -> Option<&SelectedResourceName> {
        self.display_name.as_ref()
    }

    pub const fn access(&self) -> SelectedResourceAccess {
        self.access
    }

    pub const fn has_sandbox_grant(&self) -> bool {
        self.sandbox_grant.is_some()
    }

    pub fn into_parts(
        self,
    ) -> (
        SelectedResourceKind,
        ExternalUri,
        Option<SelectedResourceName>,
        SelectedResourceAccess,
        Option<SandboxAccessGrantHandle>,
    ) {
        (
            self.kind,
            self.locator,
            self.display_name,
            self.access,
            self.sandbox_grant,
        )
    }
}

impl fmt::Debug for SelectedResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedResource")
            .field("kind", &self.kind)
            .field("locator", &self.locator)
            .field("display_name", &self.display_name)
            .field("access", &self.access)
            .field("has_sandbox_grant", &self.sandbox_grant.is_some())
            .finish_non_exhaustive()
    }
}

/// Selected-resource collection validated against exact dialog options.
#[derive(Debug)]
pub struct FileDialogSelection {
    view: ViewId,
    mode: FileDialogMode,
    resources: Vec<SelectedResource>,
}

impl FileDialogSelection {
    pub fn new(
        view: ViewId,
        options: &FileDialogOptions,
        resources: Vec<SelectedResource>,
    ) -> Result<Self, FileDialogSelectionError> {
        if resources.is_empty() {
            return Err(FileDialogSelectionError::Empty);
        }
        if resources.len() > MAX_SELECTED_RESOURCES {
            return Err(FileDialogSelectionError::TooManyResources);
        }
        if resources.len() > options.selection_limit.get() as usize {
            return Err(FileDialogSelectionError::ExceedsRequestLimit {
                supplied: resources.len(),
                maximum: options.selection_limit,
            });
        }
        let expected_kind = match options.mode {
            FileDialogMode::OpenFile | FileDialogMode::SaveFile => SelectedResourceKind::File,
            FileDialogMode::SelectFolder => SelectedResourceKind::Folder,
        };
        for (index, resource) in resources.iter().enumerate() {
            if resource.kind != expected_kind {
                return Err(FileDialogSelectionError::KindMismatch { index });
            }
            let access_is_valid = match options.mode {
                FileDialogMode::OpenFile | FileDialogMode::SelectFolder => {
                    resource.access.supports_read()
                }
                FileDialogMode::SaveFile => resource.access.supports_write(),
            };
            if !access_is_valid {
                return Err(FileDialogSelectionError::AccessMismatch { index });
            }
            if options.sandbox_access == SandboxAccessPolicy::RequireGrant
                && resource.sandbox_grant.is_none()
            {
                return Err(FileDialogSelectionError::SandboxGrantMissing { index });
            }
            if resources[..index]
                .iter()
                .any(|existing| existing.locator == resource.locator)
            {
                return Err(FileDialogSelectionError::DuplicateLocator { index });
            }
        }
        Ok(Self {
            view,
            mode: options.mode,
            resources,
        })
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn mode(&self) -> FileDialogMode {
        self.mode
    }

    pub fn resources(&self) -> &[SelectedResource] {
        &self.resources
    }

    pub fn into_resources(self) -> Vec<SelectedResource> {
        self.resources
    }
}

/// Invalid selected-resource result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogSelectionError {
    Empty,
    TooManyResources,
    ExceedsRequestLimit {
        supplied: usize,
        maximum: NonZeroU16,
    },
    KindMismatch {
        index: usize,
    },
    AccessMismatch {
        index: usize,
    },
    SandboxGrantMissing {
        index: usize,
    },
    DuplicateLocator {
        index: usize,
    },
}

impl fmt::Display for FileDialogSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "file dialog selected no resources",
            Self::TooManyResources => "file-dialog result exceeds the resource-count bound",
            Self::ExceedsRequestLimit { .. } => "file-dialog result exceeds the request limit",
            Self::KindMismatch { .. } => "selected resource kind does not match dialog mode",
            Self::AccessMismatch { .. } => "selected resource access does not match dialog mode",
            Self::SandboxGrantMissing { .. } => "selected resource lacks required sandbox access",
            Self::DuplicateLocator { .. } => "file-dialog result repeats a resource locator",
        })
    }
}

impl Error for FileDialogSelectionError {}

/// Applied dialog outcome. User dismissal is not request cancellation.
#[derive(Debug)]
pub enum FileDialogResult {
    Dismissed { view: ViewId, mode: FileDialogMode },
    Selected(FileDialogSelection),
}

impl FileDialogResult {
    pub const fn dismissed(view: ViewId, mode: FileDialogMode) -> Self {
        Self::Dismissed { view, mode }
    }

    pub const fn view(&self) -> ViewId {
        match self {
            Self::Dismissed { view, .. } => *view,
            Self::Selected(selection) => selection.view,
        }
    }

    pub const fn mode(&self) -> FileDialogMode {
        match self {
            Self::Dismissed { mode, .. } => *mode,
            Self::Selected(selection) => selection.mode,
        }
    }

    pub const fn selection(&self) -> Option<&FileDialogSelection> {
        match self {
            Self::Dismissed { .. } => None,
            Self::Selected(selection) => Some(selection),
        }
    }

    pub const fn is_dismissed(&self) -> bool {
        matches!(self, Self::Dismissed { .. })
    }
}

/// One view-scoped dialog request with an optional opaque user-gesture grant.
pub struct FileDialogRequest {
    view: ViewId,
    options: FileDialogOptions,
    user_gesture: Option<UserGestureGrantHandle>,
}

impl FileDialogRequest {
    pub const fn new(view: ViewId, options: FileDialogOptions) -> Self {
        Self {
            view,
            options,
            user_gesture: None,
        }
    }

    pub fn with_user_gesture(
        view: ViewId,
        options: FileDialogOptions,
        user_gesture: UserGestureGrantHandle,
    ) -> Self {
        Self {
            view,
            options,
            user_gesture: Some(user_gesture),
        }
    }

    pub const fn view(&self) -> ViewId {
        self.view
    }

    pub const fn options(&self) -> &FileDialogOptions {
        &self.options
    }

    pub const fn has_user_gesture(&self) -> bool {
        self.user_gesture.is_some()
    }

    pub fn into_parts(self) -> (ViewId, FileDialogOptions, Option<UserGestureGrantHandle>) {
        (self.view, self.options, self.user_gesture)
    }
}

impl fmt::Debug for FileDialogRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileDialogRequest")
            .field("view", &self.view)
            .field("options", &self.options)
            .field("has_user_gesture", &self.user_gesture.is_some())
            .finish_non_exhaustive()
    }
}

/// Immediate rejection before a dialog request is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogAdmissionError {
    ViewUnavailable { view: ViewId },
    UnsupportedMode { mode: FileDialogMode },
    MultipleSelectionUnsupported,
    SandboxGrantUnavailable,
    PermissionDenied,
    UserGestureRequired,
    InvalidUserGesture,
    FiltersExceedCapability,
    FilterRulesExceedCapability,
    SelectionsExceedCapability,
    SuggestedNameExceedsCapability,
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for FileDialogAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ViewUnavailable { .. } => "file-dialog view is unavailable",
            Self::UnsupportedMode { .. } => "file-dialog mode is unsupported",
            Self::MultipleSelectionUnsupported => "multiple file-dialog selection is unsupported",
            Self::SandboxGrantUnavailable => "sandbox access grants are unavailable",
            Self::PermissionDenied => "file-dialog permission is denied",
            Self::UserGestureRequired => "file dialog requires a recent user gesture",
            Self::InvalidUserGesture => "file dialog received an invalid user gesture",
            Self::FiltersExceedCapability => "file-dialog filters exceed capability",
            Self::FilterRulesExceedCapability => "file-dialog filter rules exceed capability",
            Self::SelectionsExceedCapability => "file-dialog selection limit exceeds capability",
            Self::SuggestedNameExceedsCapability => "file-dialog suggested name exceeds capability",
            Self::CapabilityChanged => "file-dialog capability changed before admission",
            Self::CapacityExceeded => "file-dialog admission capacity was exceeded",
        })
    }
}

impl Error for FileDialogAdmissionError {}

pub type FileDialogAdmission = RequestAdmission<FileDialogResult, FileDialogAdmissionError>;

/// Object-safe asynchronous dialog admission boundary.
pub trait FileDialogService {
    fn capability(&self, query: FileDialogCapabilityQuery) -> Support<FileDialogCapability>;

    fn show(&self, request: FileDialogRequest) -> FileDialogAdmission;
}

pub enum FileDialogServiceKey {}

impl ServiceKey for FileDialogServiceKey {
    type Handle = Rc<dyn FileDialogService>;
}

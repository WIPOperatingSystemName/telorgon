//! Extensible storage for narrow host-supplied platform service handles.
//!
//! A service family defines a zero-sized [`ServiceKey`] and chooses its associated handle type.
//! The registry keys entries by the concrete key type, not by a global service enum or by the
//! handle's representation. This lets independent service families use the same representation
//! without creating ambiguous downcasts.
//!
//! [`ServiceRegistry`] takes ownership of each registered handle. Lookup only borrows a handle;
//! it never clones it. Replacement, removal, and rejected duplicate registration return ownership
//! of the displaced, removed, or rejected handle to the caller. Dropping the registry drops all
//! remaining handles on the context that owns the registry. Neither [`ServiceKey::Handle`] nor the
//! registry requires `Send` or `Sync`, so a host may preserve owner-thread handles such as `Rc`.

use std::any::{Any, TypeId};
use std::fmt;
use std::marker::PhantomData;

pub mod accessibility;
pub mod clipboard;
pub mod cursor;
pub mod data_transfer;
pub mod display;
pub mod file_dialog;
pub mod haptics;
pub mod menu;
pub mod notification;
pub mod power;
pub mod restoration;
pub mod text_input;
pub mod uri;
pub mod window;

pub use accessibility::{
    AccessibilityActionAdmission, AccessibilityActionAdmissionError, AccessibilityActionEvent,
    AccessibilityAdmissionError, AccessibilityApplied, AccessibilityCapability,
    AccessibilityCapabilityQuery, AccessibilityLimitError, AccessibilityLimits,
    AccessibilityOperations, AccessibilityPublicationAdmission, AccessibilityPublicationRequest,
    AccessibilityService, AccessibilityServiceKey,
};
pub use clipboard::{
    ClipboardAdmissionError, ClipboardCapabilities, ClipboardCapability, ClipboardCapabilityError,
    ClipboardChange, ClipboardClearApplied, ClipboardClearRequest, ClipboardKind,
    ClipboardLimitError, ClipboardLimits, ClipboardOperations, ClipboardPublishApplied,
    ClipboardPublishRequest, ClipboardRequestAdmission, ClipboardRequestError, ClipboardRevision,
    ClipboardService, ClipboardServiceKey, ClipboardSnapshot, ClipboardSnapshotError,
    ClipboardSnapshotId, ClipboardSnapshotStatus, MAX_CLIPBOARD_CAPABILITY_FORMATS,
};
pub use cursor::{
    CursorAdmissionError, CursorAnimationFrame, CursorAppearance, CursorAppearanceAdmission,
    CursorAppearanceApplied, CursorAppearanceRequest, CursorCapability, CursorCapabilityQuery,
    CursorConstraintAdmission, CursorConstraintKind, CursorConstraintLease,
    CursorConstraintLeaseHandle, CursorConstraintLeaseStatus, CursorConstraintRequest,
    CursorConstraintRevocation, CursorImageError, CursorLimitError, CursorLimits, CursorOperations,
    CursorPositionAdmission, CursorPositionApplied, CursorPositionError, CursorPositionRequest,
    CursorSelection, CursorSelectionKind, CursorService, CursorServiceKey, CustomCursor,
    CustomCursorAnimation, CustomCursorImage, MAX_CUSTOM_CURSOR_ANIMATION_BYTES,
    MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS, MAX_CUSTOM_CURSOR_DIMENSION,
    MAX_CUSTOM_CURSOR_FRAME_DURATION_MS, MAX_CUSTOM_CURSOR_FRAMES, MAX_CUSTOM_CURSOR_IMAGE_BYTES,
    PointerIcon, StandardCursor,
};
pub use data_transfer::{
    DataFormat, DataFormatError, DataFormatKind, DataFormatReadRequest, DataOfferDescriptor,
    DataOfferError, DataReadAdmission, DataReadCompletion, DataReadMetadataError, DataReadMode,
    DataReadProgress, DataReadValidationError, DataSourceKind, DataTransferAdmissionError,
    DataTransferCapability, DataTransferLimitError, DataTransferLimits, DataTransferOperations,
    DataTransferService, DataTransferServiceKey, MAX_DATA_FORMAT_IDENTIFIER_BYTES,
    MAX_DATA_FORMATS_PER_OFFER, MAX_DATA_READ_BYTES, MAX_DATA_STREAM_CHUNK_BYTES, SizeHint,
    TrustLevel,
};
pub use display::{
    DisplayAccuracy, DisplayAccuracyProfile, DisplayCapability, DisplayChange, DisplayChangeError,
    DisplayDescriptor, DisplayDescriptorError, DisplayLimitError, DisplayLimits,
    DisplayLogicalBounds, DisplayOperations, DisplayRevision, DisplayService, DisplayServiceKey,
    DisplaySnapshot, DisplaySnapshotError, DisplaySnapshotStatus, MAX_DISPLAYS, ViewDisplayError,
    ViewDisplaySnapshot, ViewDisplayStatus,
};
pub use file_dialog::{
    FileDialogAdmission, FileDialogAdmissionError, FileDialogCapability, FileDialogCapabilityQuery,
    FileDialogFilter, FileDialogFilterError, FileDialogFilterRule, FileDialogLimitError,
    FileDialogLimits, FileDialogMode, FileDialogOperations, FileDialogOptions,
    FileDialogOptionsError, FileDialogRequest, FileDialogResult, FileDialogSelection,
    FileDialogSelectionError, FileDialogService, FileDialogServiceKey, FileExtension,
    FileExtensionError, MAX_FILE_DIALOG_FILTER_LABEL_BYTES, MAX_FILE_DIALOG_FILTER_RULES,
    MAX_FILE_DIALOG_FILTERS, MAX_FILE_EXTENSION_BYTES, MAX_SELECTED_RESOURCE_NAME_BYTES,
    MAX_SELECTED_RESOURCES, MAX_SUGGESTED_FILE_NAME_BYTES, SandboxAccessGrant,
    SandboxAccessGrantHandle, SandboxAccessPolicy, SelectedResource, SelectedResourceAccess,
    SelectedResourceKind, SelectedResourceName, SelectedResourceNameError, SuggestedFileName,
    SuggestedFileNameError,
};
pub use haptics::{
    HAPTIC_INTENSITY_UNITS, HapticAdmission, HapticAdmissionError, HapticApplied, HapticCapability,
    HapticCapabilityError, HapticDeviceSupport, HapticDeviceSupportError, HapticEffect,
    HapticEffectSupport, HapticIntensity, HapticIntensityError, HapticLimitError, HapticLimits,
    HapticOperations, HapticRequest, HapticUserSettingState, HapticsService, HapticsServiceKey,
};
pub use menu::{
    MAX_MENU_ACCELERATOR_LABEL_BYTES, MAX_MENU_ACCELERATORS, MAX_MENU_DEPTH, MAX_MENU_ITEMS,
    MAX_MENU_LABEL_BYTES, MenuAccelerator, MenuAcceleratorError, MenuAcceleratorLabel,
    MenuActionAdmission, MenuActionAdmissionError, MenuActionEvent, MenuActionRequest,
    MenuActionSource, MenuAdmissionError, MenuCapability, MenuCapabilityQuery, MenuCheckState,
    MenuItem, MenuItemError, MenuItemId, MenuItemKind, MenuItemState, MenuLabel, MenuLimitError,
    MenuLimits, MenuOperations, MenuPublicationAdmission, MenuPublicationApplied,
    MenuPublicationError, MenuPublicationRequest, MenuRevision, MenuRole, MenuScope, MenuService,
    MenuServiceKey, MenuSnapshotId, MenuTextError, MenuTree, MenuTreeError, StatusMenuId,
};
pub use notification::{
    MAX_NOTIFICATION_ACTION_LABEL_BYTES, MAX_NOTIFICATION_ACTIONS, MAX_NOTIFICATION_BADGE_COUNT,
    MAX_NOTIFICATION_BODY_BYTES, MAX_NOTIFICATION_REPLY_BYTES, MAX_NOTIFICATION_TITLE_BYTES,
    NotificationAction, NotificationActionError, NotificationActionId, NotificationActionKind,
    NotificationActionLabel, NotificationAdmissionError, NotificationAuthorizationAdmission,
    NotificationAuthorizationApplied, NotificationAuthorizationOptions,
    NotificationAuthorizationOptionsError, NotificationAuthorizationRequest, NotificationBadge,
    NotificationBadgeAdmission, NotificationBadgeApplied, NotificationBadgeError,
    NotificationBadgeRequest, NotificationBody, NotificationCapability, NotificationDescriptor,
    NotificationDescriptorError, NotificationId, NotificationLimitError, NotificationLimits,
    NotificationOperations, NotificationPriority, NotificationPrivacy,
    NotificationPublicationAdmission, NotificationPublicationApplied, NotificationPublicationError,
    NotificationPublicationRequest, NotificationRemovalAdmission, NotificationRemovalApplied,
    NotificationRemovalRequest, NotificationReply, NotificationResponseAdmission,
    NotificationResponseAdmissionError, NotificationResponseEvent, NotificationResponseRequest,
    NotificationResponseSource, NotificationRevision, NotificationService, NotificationServiceKey,
    NotificationSnapshotId, NotificationTextError, NotificationTitle,
};
pub use power::{
    MAX_POWER_INHIBITION_LEASES, PowerAdmissionError, PowerCapability, PowerCapabilityQuery,
    PowerInhibitionAdmission, PowerInhibitionKind, PowerInhibitionLease,
    PowerInhibitionLeaseHandle, PowerInhibitionLeaseStatus, PowerInhibitionReason,
    PowerInhibitionRequest, PowerInhibitionRevocation, PowerInhibitionScope, PowerLimitError,
    PowerLimits, PowerOperations, PowerPolicyState, PowerService, PowerServiceKey,
};
pub use restoration::{
    MAX_RESTORATION_TOKEN_BYTES, RestorationAdmissionError, RestorationCapability,
    RestorationCapabilityQuery, RestorationClearAdmission, RestorationClearApplied,
    RestorationClearRequest, RestorationConsumptionAdmission, RestorationConsumptionApplied,
    RestorationConsumptionRequest, RestorationLimitError, RestorationLimits, RestorationOperations,
    RestorationPublicationAdmission, RestorationPublicationApplied, RestorationPublicationError,
    RestorationPublicationRequest, RestorationRecord, RestorationRevision, RestorationScope,
    RestorationService, RestorationServiceKey, RestorationSnapshotId, RestorationToken,
    RestorationTokenError,
};
pub use text_input::{
    MAX_TEXT_INPUT_SURROUNDING_BYTES, TextInputAdmission, TextInputAdmissionError,
    TextInputApplied, TextInputCapability, TextInputCapabilityQuery, TextInputDeltaEvent,
    TextInputDeltaKind, TextInputLimitError, TextInputLimits, TextInputOperations,
    TextInputService, TextInputServiceKey, TextInputSyncError, TextInputSyncKind,
    TextInputSyncRequest,
};
pub use uri::{
    ExternalUri, ExternalUriError, MAX_EXTERNAL_URI_BYTES, MAX_URI_SCHEME_BYTES, MAX_URI_SCHEMES,
    UriAdmissionError, UriCapabilities, UriCapability, UriCapabilityError, UriLimitError,
    UriLimits, UriOpenAdmission, UriOpenApplied, UriOpenRequest, UriOperation, UriScheme,
    UriSchemeCapability, UriSchemeError, UriService, UriServiceKey,
};
pub use window::{
    MAX_WINDOW_TITLE_BYTES, WindowAdmissionError, WindowAttentionApplied, WindowAttentionIntent,
    WindowAttentionRequest, WindowCapability, WindowCapabilityLimits, WindowCapabilityQuery,
    WindowCloseApplied, WindowCloseIntent, WindowCloseRequest, WindowConstraintAxis,
    WindowConstraintBound, WindowOperation, WindowRequestAdmission, WindowService,
    WindowServiceKey, WindowSizeConstraints, WindowSizeConstraintsApplied,
    WindowSizeConstraintsError, WindowSizeConstraintsRequest, WindowStateApplied,
    WindowStateIntent, WindowStateRequest, WindowTitle, WindowTitleApplied, WindowTitleError,
    WindowTitleRequest,
};

/// A unique type-level key for one narrow platform service family.
///
/// Implementations are normally uninhabited enums or zero-sized structs. The concrete key type is
/// the registry identity, while [`Handle`](Self::Handle) is the exact host-owned handle exposed by
/// lookup. Defining a new service requires only a new key implementation; it does not modify the
/// registry or a shared command enum.
pub trait ServiceKey: 'static {
    /// Exact handle representation retained for this service family.
    ///
    /// The representation may be a concrete owner, `Rc<dyn Trait>`, `Arc<dyn Trait>`, or another
    /// host-selected `'static` value. No cross-thread traits are imposed by the registry.
    type Handle: 'static;
}

/// Why a requested service handle is unavailable from a registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceUnavailable {
    /// No handle was registered for the requested concrete [`ServiceKey`].
    NotRegistered,
}

/// Result of borrowing a service handle.
#[derive(Debug)]
#[must_use = "service absence must be handled explicitly"]
pub enum ServiceLookup<'a, Handle> {
    /// The exact handle registered for the requested key.
    Available(&'a Handle),
    /// The registry contains no handle for the requested key.
    Unavailable(ServiceUnavailable),
}

impl<'a, Handle> ServiceLookup<'a, Handle> {
    /// Returns whether this lookup found a registered handle.
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Returns the unavailable classification, if lookup found no handle.
    pub const fn unavailable_reason(&self) -> Option<ServiceUnavailable> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }

    /// Converts the result into the borrowed handle, discarding only the absence classification.
    pub const fn into_available(self) -> Option<&'a Handle> {
        match self {
            Self::Available(handle) => Some(handle),
            Self::Unavailable(_) => None,
        }
    }
}

/// Result of registering a handle without replacing an existing one.
#[derive(Debug)]
#[must_use = "duplicate registration returns ownership of the rejected handle"]
pub enum ServiceRegistration<Handle> {
    /// The registry took ownership of the handle.
    Registered,
    /// The key was already occupied and the existing handle was left unchanged.
    AlreadyRegistered {
        /// The unregistered handle returned to the caller.
        rejected: Handle,
    },
}

impl<Handle> ServiceRegistration<Handle> {
    /// Returns whether the registry took ownership of the supplied handle.
    pub const fn is_registered(&self) -> bool {
        matches!(self, Self::Registered)
    }

    /// Returns a handle rejected because the key was already registered.
    pub fn into_rejected(self) -> Option<Handle> {
        match self {
            Self::Registered => None,
            Self::AlreadyRegistered { rejected } => Some(rejected),
        }
    }
}

/// Result of deterministically installing a handle, replacing any prior value.
#[derive(Debug)]
#[must_use = "replacement can return ownership of a previous handle"]
pub enum ServiceReplacement<Handle> {
    /// The key was absent and the registry inserted the handle.
    Inserted,
    /// The new handle was installed and the previous handle is returned to the caller.
    Replaced {
        /// The formerly registered handle.
        previous: Handle,
    },
}

impl<Handle> ServiceReplacement<Handle> {
    /// Returns the displaced handle when replacement found an occupied key.
    pub fn into_previous(self) -> Option<Handle> {
        match self {
            Self::Inserted => None,
            Self::Replaced { previous } => Some(previous),
        }
    }
}

/// Result of removing a service handle.
#[derive(Debug)]
#[must_use = "removal distinguishes an absent service and returns removed ownership"]
pub enum ServiceRemoval<Handle> {
    /// The registered handle was removed and is returned to the caller.
    Removed(Handle),
    /// The requested key was not registered; no fallback handle was constructed.
    Unavailable(ServiceUnavailable),
}

impl<Handle> ServiceRemoval<Handle> {
    /// Returns the removed handle, if the key was registered.
    pub fn into_removed(self) -> Option<Handle> {
        match self {
            Self::Removed(handle) => Some(handle),
            Self::Unavailable(_) => None,
        }
    }

    /// Returns the unavailable classification when no handle was removed.
    pub const fn unavailable_reason(&self) -> Option<ServiceUnavailable> {
        match self {
            Self::Removed(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }
}

struct StoredService<Key: ServiceKey> {
    handle: Key::Handle,
    key: PhantomData<fn() -> Key>,
}

impl<Key: ServiceKey> StoredService<Key> {
    fn new(handle: Key::Handle) -> Self {
        Self {
            handle,
            key: PhantomData,
        }
    }
}

struct ErasedService {
    key: TypeId,
    stored: Box<dyn Any>,
}

/// Owner of a small heterogeneous set of host-supplied platform service handles.
///
/// The registry exposes no iteration or invocation policy. Its only behavior is exact typed
/// registration, replacement, removal, and borrowing. In particular, lookup of an absent key
/// returns [`ServiceLookup::Unavailable`] and never constructs a native or fallback service.
#[derive(Default)]
pub struct ServiceRegistry {
    entries: Vec<ErasedService>,
}

impl ServiceRegistry {
    /// Creates an empty registry with no implicit or fallback services.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Returns the number of concrete service keys currently registered.
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no service handles are registered.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns whether a handle is registered for `Key` without borrowing or cloning it.
    pub fn contains<Key: ServiceKey>(&self) -> bool {
        self.entry_index::<Key>().is_some()
    }

    /// Registers `handle` only when `Key` is absent.
    ///
    /// Duplicate registration leaves the current entry unchanged and returns the supplied handle
    /// in [`ServiceRegistration::AlreadyRegistered`].
    pub fn register<Key: ServiceKey>(
        &mut self,
        handle: Key::Handle,
    ) -> ServiceRegistration<Key::Handle> {
        if self.contains::<Key>() {
            return ServiceRegistration::AlreadyRegistered { rejected: handle };
        }

        self.entries.push(ErasedService {
            key: TypeId::of::<Key>(),
            stored: Box::new(StoredService::<Key>::new(handle)),
        });
        ServiceRegistration::Registered
    }

    /// Installs `handle`, returning the previous value when `Key` was already registered.
    pub fn replace<Key: ServiceKey>(
        &mut self,
        handle: Key::Handle,
    ) -> ServiceReplacement<Key::Handle> {
        let Some(index) = self.entry_index::<Key>() else {
            let registration = self.register::<Key>(handle);
            debug_assert!(registration.is_registered());
            return ServiceReplacement::Inserted;
        };

        let previous = std::mem::replace(
            &mut self.entries[index].stored,
            Box::new(StoredService::<Key>::new(handle)),
        );
        ServiceReplacement::Replaced {
            previous: Self::into_typed::<Key>(previous).handle,
        }
    }

    /// Borrows the exact handle registered for `Key`.
    pub fn lookup<Key: ServiceKey>(&self) -> ServiceLookup<'_, Key::Handle> {
        let Some(index) = self.entry_index::<Key>() else {
            return ServiceLookup::Unavailable(ServiceUnavailable::NotRegistered);
        };

        ServiceLookup::Available(&Self::as_typed::<Key>(&self.entries[index]).handle)
    }

    /// Removes and returns the exact handle registered for `Key`.
    pub fn remove<Key: ServiceKey>(&mut self) -> ServiceRemoval<Key::Handle> {
        let Some(index) = self.entry_index::<Key>() else {
            return ServiceRemoval::Unavailable(ServiceUnavailable::NotRegistered);
        };

        let erased = self.entries.remove(index).stored;
        ServiceRemoval::Removed(Self::into_typed::<Key>(erased).handle)
    }

    fn entry_index<Key: ServiceKey>(&self) -> Option<usize> {
        let key = TypeId::of::<Key>();
        self.entries.iter().position(|entry| entry.key == key)
    }

    fn as_typed<Key: ServiceKey>(entry: &ErasedService) -> &StoredService<Key> {
        // `stored` is written only together with the same private `Key` type ID. Keeping the key
        // and value behind private fields makes a mismatched entry unconstructable by callers.
        entry
            .stored
            .downcast_ref::<StoredService<Key>>()
            .expect("private service entry type invariant violated")
    }

    fn into_typed<Key: ServiceKey>(stored: Box<dyn Any>) -> Box<StoredService<Key>> {
        // See `as_typed`: every insertion and replacement records the concrete key and its exact
        // `StoredService<Key>` value in one operation.
        stored
            .downcast::<StoredService<Key>>()
            .expect("private service entry type invariant violated")
    }
}

impl fmt::Debug for ServiceRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceRegistry")
            .field("registered_service_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    trait CounterService {
        fn value(&self) -> u32;
    }

    struct LocalCounter(Rc<Cell<u32>>);

    impl CounterService for LocalCounter {
        fn value(&self) -> u32 {
            self.0.get()
        }
    }

    enum PrimaryCounter {}

    impl ServiceKey for PrimaryCounter {
        type Handle = Rc<dyn CounterService>;
    }

    enum SecondaryCounter {}

    impl ServiceKey for SecondaryCounter {
        type Handle = Rc<dyn CounterService>;
    }

    fn counter(value: u32) -> Rc<dyn CounterService> {
        Rc::new(LocalCounter(Rc::new(Cell::new(value))))
    }

    #[test]
    fn absent_services_are_explicit_and_never_fabricated() {
        let mut registry = ServiceRegistry::new();

        assert!(registry.is_empty());
        assert_eq!(
            registry.lookup::<PrimaryCounter>().unavailable_reason(),
            Some(ServiceUnavailable::NotRegistered)
        );
        assert_eq!(
            registry.remove::<PrimaryCounter>().unavailable_reason(),
            Some(ServiceUnavailable::NotRegistered)
        );
        assert!(registry.is_empty());
    }

    #[test]
    fn registration_rejects_duplicates_without_mutating_the_existing_handle() {
        let mut registry = ServiceRegistry::new();
        assert!(
            registry
                .register::<PrimaryCounter>(counter(7))
                .is_registered()
        );

        let rejected = registry
            .register::<PrimaryCounter>(counter(99))
            .into_rejected()
            .expect("duplicate handle must be returned");

        assert_eq!(rejected.value(), 99);
        assert_eq!(
            registry
                .lookup::<PrimaryCounter>()
                .into_available()
                .unwrap()
                .value(),
            7
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn key_identity_disambiguates_identical_handle_representations() {
        let mut registry = ServiceRegistry::new();
        assert!(
            registry
                .register::<PrimaryCounter>(counter(3))
                .is_registered()
        );
        assert!(
            registry
                .register::<SecondaryCounter>(counter(41))
                .is_registered()
        );

        assert_eq!(
            registry
                .lookup::<PrimaryCounter>()
                .into_available()
                .unwrap()
                .value(),
            3
        );
        assert_eq!(
            registry
                .lookup::<SecondaryCounter>()
                .into_available()
                .unwrap()
                .value(),
            41
        );
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn replacement_and_removal_transfer_owned_handles_deterministically() {
        let mut registry = ServiceRegistry::new();

        assert!(matches!(
            registry.replace::<PrimaryCounter>(counter(5)),
            ServiceReplacement::Inserted
        ));
        let previous = registry
            .replace::<PrimaryCounter>(counter(8))
            .into_previous()
            .expect("occupied replacement must return the old handle");
        assert_eq!(previous.value(), 5);
        assert_eq!(
            registry
                .lookup::<PrimaryCounter>()
                .into_available()
                .unwrap()
                .value(),
            8
        );

        let removed = registry
            .remove::<PrimaryCounter>()
            .into_removed()
            .expect("registered handle must be removed");
        assert_eq!(removed.value(), 8);
        assert!(!registry.contains::<PrimaryCounter>());
        assert!(registry.is_empty());
    }

    #[test]
    fn lookup_borrows_without_cloning_or_imposing_cross_thread_traits() {
        let state = Rc::new(Cell::new(12));
        let handle: Rc<dyn CounterService> = Rc::new(LocalCounter(Rc::clone(&state)));
        let mut registry = ServiceRegistry::new();
        assert_eq!(Rc::strong_count(&handle), 1);

        assert!(registry.register::<PrimaryCounter>(handle).is_registered());
        let borrowed = registry
            .lookup::<PrimaryCounter>()
            .into_available()
            .unwrap();
        assert_eq!(borrowed.value(), 12);
        assert_eq!(Rc::strong_count(borrowed), 1);

        state.set(27);
        assert_eq!(borrowed.value(), 27);
    }
}

//! Neutral platform identities and contracts.
//!
//! Native handles, event-loop objects, protocol identifiers, renderer resources, and service
//! implementations belong to adapter or backend packages rather than this crate.

pub mod capability;
pub mod clock;
pub mod error;
pub mod event;
pub mod id;
pub mod lifecycle;
pub mod metrics;
pub mod request;
pub mod schedule;
pub mod services;
pub mod stamp;
pub mod view;

pub use capability::{
    CapabilityDescriptor, CapabilityLimit, ExecutionRequirement, NoCapabilityLimits,
    PermissionState, Support, UnavailableReason, UserGestureGrant, UserGestureGrantHandle,
    UserGestureRequirement,
};
pub use clock::{MonotonicClock, MonotonicClockError, MonotonicClockState};
pub use error::{PlatformError, PlatformErrorKind, PlatformErrorSource, PlatformResult};
pub use event::{CoalescingMetadata, CollapsedEventCount, MetricsCitation, PlatformEvent};
pub use id::{
    CursorConstraintLeaseId, DataOfferId, DisplayId, NativeSurfaceGeneration,
    PowerInhibitionLeaseId, RequestId, RestorationSessionId, ViewId,
};
pub use lifecycle::{
    ActivityState, LifecycleAxis, LifecycleError, LifecycleTransition, NativeSurfaceState,
    ViewLifecycle, ViewLifetime, VisibilityState,
};
pub use metrics::{
    AvoidRegion, AvoidRegionKind, CoordinateSpace, DisplayColorSpace, DisplayOrientation,
    DisplayProperties, DisplayTransform, HdrState, InsetKind, LogicalToPhysicalTransform,
    MAX_AVOID_REGIONS, MetricInsets, MetricsRevision, PhysicalExtent, ScaleFactor, ViewMetrics,
    ViewMetricsError, ViewMetricsSnapshot, ViewMetricsState, ViewMetricsUpdate,
};
pub use request::{AdmittedRequest, RequestAdmission, RequestCompletion, RequestOutcome};
pub use schedule::{
    MAX_REDRAW_VIEWS, PendingHostFacts, PostTurnSchedule, RemainingWork, ScheduleError,
};
pub use services::{
    AccessibilityActionAdmission, AccessibilityActionAdmissionError, AccessibilityActionEvent,
    AccessibilityAdmissionError, AccessibilityApplied, AccessibilityCapability,
    AccessibilityCapabilityQuery, AccessibilityLimitError, AccessibilityLimits,
    AccessibilityOperations, AccessibilityPublicationAdmission, AccessibilityPublicationRequest,
    AccessibilityService, AccessibilityServiceKey, ClipboardAdmissionError, ClipboardCapabilities,
    ClipboardCapability, ClipboardCapabilityError, ClipboardChange, ClipboardClearApplied,
    ClipboardClearRequest, ClipboardKind, ClipboardLimitError, ClipboardLimits,
    ClipboardOperations, ClipboardPublishApplied, ClipboardPublishRequest,
    ClipboardRequestAdmission, ClipboardRequestError, ClipboardRevision, ClipboardService,
    ClipboardServiceKey, ClipboardSnapshot, ClipboardSnapshotError, ClipboardSnapshotId,
    ClipboardSnapshotStatus, CursorAdmissionError, CursorAnimationFrame, CursorAppearance,
    CursorAppearanceAdmission, CursorAppearanceApplied, CursorAppearanceRequest, CursorCapability,
    CursorCapabilityQuery, CursorConstraintAdmission, CursorConstraintKind, CursorConstraintLease,
    CursorConstraintLeaseHandle, CursorConstraintLeaseStatus, CursorConstraintRequest,
    CursorConstraintRevocation, CursorImageError, CursorLimitError, CursorLimits, CursorOperations,
    CursorPositionAdmission, CursorPositionApplied, CursorPositionError, CursorPositionRequest,
    CursorSelection, CursorSelectionKind, CursorService, CursorServiceKey, CustomCursor,
    CustomCursorAnimation, CustomCursorImage, DataFormat, DataFormatError, DataFormatKind,
    DataFormatReadRequest, DataOfferDescriptor, DataOfferError, DataReadAdmission,
    DataReadCompletion, DataReadMetadataError, DataReadMode, DataReadProgress,
    DataReadValidationError, DataSourceKind, DataTransferAdmissionError, DataTransferCapability,
    DataTransferLimitError, DataTransferLimits, DataTransferOperations, DataTransferService,
    DataTransferServiceKey, DisplayAccuracy, DisplayAccuracyProfile, DisplayCapability,
    DisplayChange, DisplayChangeError, DisplayDescriptor, DisplayDescriptorError,
    DisplayLimitError, DisplayLimits, DisplayLogicalBounds, DisplayOperations, DisplayRevision,
    DisplayService, DisplayServiceKey, DisplaySnapshot, DisplaySnapshotError,
    DisplaySnapshotStatus, ExternalUri, ExternalUriError, FileDialogAdmission,
    FileDialogAdmissionError, FileDialogCapability, FileDialogCapabilityQuery, FileDialogFilter,
    FileDialogFilterError, FileDialogFilterRule, FileDialogLimitError, FileDialogLimits,
    FileDialogMode, FileDialogOperations, FileDialogOptions, FileDialogOptionsError,
    FileDialogRequest, FileDialogResult, FileDialogSelection, FileDialogSelectionError,
    FileDialogService, FileDialogServiceKey, FileExtension, FileExtensionError,
    HAPTIC_INTENSITY_UNITS, HapticAdmission, HapticAdmissionError, HapticApplied, HapticCapability,
    HapticCapabilityError, HapticDeviceSupport, HapticDeviceSupportError, HapticEffect,
    HapticEffectSupport, HapticIntensity, HapticIntensityError, HapticLimitError, HapticLimits,
    HapticOperations, HapticRequest, HapticUserSettingState, HapticsService, HapticsServiceKey,
    MAX_CLIPBOARD_CAPABILITY_FORMATS, MAX_CUSTOM_CURSOR_ANIMATION_BYTES,
    MAX_CUSTOM_CURSOR_ANIMATION_DURATION_MS, MAX_CUSTOM_CURSOR_DIMENSION,
    MAX_CUSTOM_CURSOR_FRAME_DURATION_MS, MAX_CUSTOM_CURSOR_FRAMES, MAX_CUSTOM_CURSOR_IMAGE_BYTES,
    MAX_DATA_FORMAT_IDENTIFIER_BYTES, MAX_DATA_FORMATS_PER_OFFER, MAX_DATA_READ_BYTES,
    MAX_DATA_STREAM_CHUNK_BYTES, MAX_DISPLAYS, MAX_EXTERNAL_URI_BYTES,
    MAX_FILE_DIALOG_FILTER_LABEL_BYTES, MAX_FILE_DIALOG_FILTER_RULES, MAX_FILE_DIALOG_FILTERS,
    MAX_FILE_EXTENSION_BYTES, MAX_MENU_ACCELERATOR_LABEL_BYTES, MAX_MENU_ACCELERATORS,
    MAX_MENU_DEPTH, MAX_MENU_ITEMS, MAX_MENU_LABEL_BYTES, MAX_NOTIFICATION_ACTION_LABEL_BYTES,
    MAX_NOTIFICATION_ACTIONS, MAX_NOTIFICATION_BADGE_COUNT, MAX_NOTIFICATION_BODY_BYTES,
    MAX_NOTIFICATION_REPLY_BYTES, MAX_NOTIFICATION_TITLE_BYTES, MAX_POWER_INHIBITION_LEASES,
    MAX_RESTORATION_TOKEN_BYTES, MAX_SELECTED_RESOURCE_NAME_BYTES, MAX_SELECTED_RESOURCES,
    MAX_SUGGESTED_FILE_NAME_BYTES, MAX_TEXT_INPUT_SURROUNDING_BYTES, MAX_URI_SCHEME_BYTES,
    MAX_URI_SCHEMES, MAX_WINDOW_TITLE_BYTES, MenuAccelerator, MenuAcceleratorError,
    MenuAcceleratorLabel, MenuActionAdmission, MenuActionAdmissionError, MenuActionEvent,
    MenuActionRequest, MenuActionSource, MenuAdmissionError, MenuCapability, MenuCapabilityQuery,
    MenuCheckState, MenuItem, MenuItemError, MenuItemId, MenuItemKind, MenuItemState, MenuLabel,
    MenuLimitError, MenuLimits, MenuOperations, MenuPublicationAdmission, MenuPublicationApplied,
    MenuPublicationError, MenuPublicationRequest, MenuRevision, MenuRole, MenuScope, MenuService,
    MenuServiceKey, MenuSnapshotId, MenuTextError, MenuTree, MenuTreeError, NotificationAction,
    NotificationActionError, NotificationActionId, NotificationActionKind, NotificationActionLabel,
    NotificationAdmissionError, NotificationAuthorizationAdmission,
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
    NotificationSnapshotId, NotificationTextError, NotificationTitle, PointerIcon,
    PowerAdmissionError, PowerCapability, PowerCapabilityQuery, PowerInhibitionAdmission,
    PowerInhibitionKind, PowerInhibitionLease, PowerInhibitionLeaseHandle,
    PowerInhibitionLeaseStatus, PowerInhibitionReason, PowerInhibitionRequest,
    PowerInhibitionRevocation, PowerInhibitionScope, PowerLimitError, PowerLimits, PowerOperations,
    PowerPolicyState, PowerService, PowerServiceKey, RestorationAdmissionError,
    RestorationCapability, RestorationCapabilityQuery, RestorationClearAdmission,
    RestorationClearApplied, RestorationClearRequest, RestorationConsumptionAdmission,
    RestorationConsumptionApplied, RestorationConsumptionRequest, RestorationLimitError,
    RestorationLimits, RestorationOperations, RestorationPublicationAdmission,
    RestorationPublicationApplied, RestorationPublicationError, RestorationPublicationRequest,
    RestorationRecord, RestorationRevision, RestorationScope, RestorationService,
    RestorationServiceKey, RestorationSnapshotId, RestorationToken, RestorationTokenError,
    SandboxAccessGrant, SandboxAccessGrantHandle, SandboxAccessPolicy, SelectedResource,
    SelectedResourceAccess, SelectedResourceKind, SelectedResourceName, SelectedResourceNameError,
    ServiceKey, ServiceLookup, ServiceRegistration, ServiceRegistry, ServiceRemoval,
    ServiceReplacement, ServiceUnavailable, SizeHint, StandardCursor, StatusMenuId,
    SuggestedFileName, SuggestedFileNameError, TextInputAdmission, TextInputAdmissionError,
    TextInputApplied, TextInputCapability, TextInputCapabilityQuery, TextInputDeltaEvent,
    TextInputDeltaKind, TextInputLimitError, TextInputLimits, TextInputOperations,
    TextInputService, TextInputServiceKey, TextInputSyncError, TextInputSyncKind,
    TextInputSyncRequest, TrustLevel, UriAdmissionError, UriCapabilities, UriCapability,
    UriCapabilityError, UriLimitError, UriLimits, UriOpenAdmission, UriOpenApplied, UriOpenRequest,
    UriOperation, UriScheme, UriSchemeCapability, UriSchemeError, UriService, UriServiceKey,
    ViewDisplayError, ViewDisplaySnapshot, ViewDisplayStatus, WindowAdmissionError,
    WindowAttentionApplied, WindowAttentionIntent, WindowAttentionRequest, WindowCapability,
    WindowCapabilityLimits, WindowCapabilityQuery, WindowCloseApplied, WindowCloseIntent,
    WindowCloseRequest, WindowConstraintAxis, WindowConstraintBound, WindowOperation,
    WindowRequestAdmission, WindowService, WindowServiceKey, WindowSizeConstraints,
    WindowSizeConstraintsApplied, WindowSizeConstraintsError, WindowSizeConstraintsRequest,
    WindowStateApplied, WindowStateIntent, WindowStateRequest, WindowTitle, WindowTitleApplied,
    WindowTitleError, WindowTitleRequest,
};
pub use stamp::{EventStamp, EventStampError, EventStampStream, MonotonicInstant};
pub use view::{
    CloseRequest, CloseRequestDecision, CloseRequestReason, ForcedDestruction,
    ForcedDestructionPhase, ViewRevision, ViewSnapshot, ViewState, ViewStateError, ViewUpdate,
};

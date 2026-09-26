//! Telorgon's consolidated retained UI framework and curated public facade.
//!
//! Subsystem ownership remains visible through focused modules while applications can import the
//! ordinary authoring surface with `use telorgon::app::*`.

pub use crate::services::session::{GuiSessionConfig, ApplicationRegistry, ApplicationSpec, ApplicationRef, ApplicationHandle, SessionApplications};

pub use crate::host::application::request_exit;
pub use crate::host::application::{
    Capture, CaptureProtocols, CaptureSources, DirectCaptureAccess, InternalCapture, PortalCapture,
    PortalSessionIntegration, WaylandCapture,
};

/// Authoring API for shell-designed screen-cast portals.
#[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
pub mod portal {
    pub use crate::host::application::{PortalPickerApplication, ReadyPortalPickerApplication, ScreenCastPortal, PortalPickerWindow, ShareAudio, AudioScope, PortalAudioDelivery, PortalAudioRequest, PortalAudioSession};
    pub use crate::authoring::compose::portal::{
        ScreenCastPortalContext, ScreenCastPortalSnapshot, SavedCapturePermission, PortalSourcePreview,
        VirtualDisplayConfig, VirtualDisplaySnapshot,
    };
}
#[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
pub use portal::*;

pub use crate::ui::text::Typography;
pub use crate::assets::{
    AppIconProfile, AppIconProfileError, AppIconVariant, AssetBundle, AssetCatalog,
    AssetResolver, AssetCatalogError, AssetEntry, AssetError, AssetKey, AssetKind, AssetMediaCache,
    AssetMediaError, AssetRasterSize, ClientCursorMode, CursorAsset, CursorGraphic, CursorTheme,
    CursorThemeAsset, CursorThemeError, DecodedAssetImage, FontAsset, Icon, IconAsset, ImageAsset,
    ImageSource, PointerConfiguration, PointerFrame, PointerHotspot, PointerRequest,
    PointerResolution, PointerTheme, PointerThemeFallback, PointerThemeOverrides,
    REQUIRED_CURSOR_ROLES, asset_image_id, cursor, resolve_pointer,
};
pub use crate::authoring::fill::{Fill, GlassStyle};
#[cfg(feature = "embedded-profiler")]
pub use crate::runtime::instrumentation as embedded_profiler_events;
pub use crate::ui::accessibility::{
    AssistiveActionData, AssistiveActionError, AssistiveActionRequest, MAX_ACTION_TEXT_BYTES,
    MAX_SEMANTIC_CHILDREN_PER_NODE, MAX_SEMANTIC_NODES, MAX_SEMANTIC_RELATIONSHIPS_PER_NODE,
    MAX_SEMANTIC_STRING_BYTES, MAX_SEMANTIC_STRINGS, MAX_SEMANTIC_TREE_STRING_BYTES,
    ResolvedSemanticString, SemanticCoordinateSpace, SemanticFocusUpdate, SemanticNodeGeometry,
    SemanticNodeId, SemanticTreeDelta, SemanticTreeError, SemanticTreeGeneration, SemanticTreeNode,
    SemanticTreePublication, SemanticTreePublicationKind, SemanticTreeRetirement,
    SemanticTreeRevision, SemanticTreeSnapshot,
};
pub use telorgon_macros::{asset_catalog, component};

pub use crate::shell::window_chrome::{
    ContentFade, DecorationNegotiation, DecorationPolicy, GeometryMotion, Minimize,
    ResizePreviewDesign, ShellActionId, Spring, WindowAction, WindowChromeCapabilities,
    WindowChromeError, WindowChromeHitSpec, WindowChromeModel, WindowChromeRegion,
    WindowChromeRole, WindowChromeSnapshot, WindowChromeState, WindowContentStyle, WindowEdgeMask,
    WindowMotion, WindowResizeEdge, WindowTilingState, WindowTween, tween_ms,
};

/// Imports shared by Telorgon's high-level application facade.
///
/// This module is intentionally private: application authors should import one of the entry-point
/// modules instead, such as `use telorgon::app::*`.
mod common {
    pub use crate::assets::fonts;
    pub use crate::authoring::compose::{
        HoverEffect, Button, Checkbox, Container, EasyWindowFrame, Image, PointerViewExt, Slider, Switch, Text,
        WindowChromeDesign, WindowChromeDesignError, WindowChromePalette, WindowChromeStateStyle,
        WindowChromeViewExt, WindowContentSlot, WindowControlButtonStyle, WindowControlDesign,
        WindowControlVisual, WindowControlsDesign, WindowFrame, WindowTitleBarStyle,
    };
    pub use crate::authoring::fill::{Fill, GlassStyle};
    pub use crate::shell::window_chrome::{
        ContentFade, GeometryMotion, Minimize, ResizePreviewDesign, Spring, WindowMotion,
        WindowTween, tween_ms,
    };
    pub use crate::{
        Typography, Alignment, AppIconProfile, AssetBundle, AssetCatalog, AssetKey, Background, Border,
        BorderSide, BoxDecoration, BoxDecorationError, BoxSizing, BoxStyle, ColorRgba8, Component,
        ComponentFields, ComponentInstanceId, CornerRadii, CrossAxisAlignment, Dimension,
        EdgeInsets, Element, EventContext, EventHandler, Flow, FontAsset, Icon, IconAsset,
        ImageAsset, ImageSource, InputsChangedContext, Insets, Key, LayoutStyle, MainAxisAlignment,
        MountContext, Outline, Overflow, PointF, RectF, Result, RuntimeTarget, SemanticCheckState,
        Shadow, ShadowList, ShellActionId, Signal, SignalSnapshot, SignalWriter, SizeF, SizeI,
        SizeRule, SizeRule2D, StyleOverride, TextStyle, Transform2D, View, ViewError, WindowAction,
        WindowChromeCapabilities, WindowChromeHitSpec, WindowChromeModel, WindowChromeRole,
        WindowChromeState, WindowContentStyle, WindowEdgeMask, WindowResizeEdge, WindowTilingState,
        asset_catalog, button, card, checkbox, column, component, easy_window_frame, hashed_key,
        image, row, slider, spacer, stack, switch, text, window_content_slot, window_frame,
    };
}

/// Complete facade for ordinary managed application authoring.
///
/// A single `use telorgon::app::*` imports the component macro and traits, composition builders,
/// common style/geometry values, the two `Application` constructors, and Telorgon's `Result` alias.
pub mod app {
    pub use super::common::*;
    pub use crate::authoring::compose::{
        ApplicationCatalog, ApplicationCatalogHandle, ApplicationCatalogStatus, ApplicationIcon,
        ApplicationId, ApplicationMetadata, ApplicationQuery, ApplicationVisibility, IconRequest,
        ShellContext, ShellRequestCompletion, ShellRequestOutcome, ShellServiceError,
        ShellServices, ShellWindow, ShellWindowAction, ShellWindows,
    };
    pub use crate::authoring::compose::{
        ShellAttachment, ShellChild, ShellDismissReason, ShellEdge, ShellExtent, ShellFocus,
        ShellOutputPreview, ShellPlacementBounds, ShellPointer, ShellReservation,
        ShellSurfaceLayer, ShellSurfaceSpec, ShellWidget, ShellWindowPreview, WidgetPlacement,
    };
    pub use crate::authoring::compose::{
        TilePreviewDesign, TilePreviewMotion, TileTarget, WindowTiling,
    };
    pub use crate::host::application::{
        Application, Compositor, KeyBindings, KeyChord, LinuxShellConfig, OutputScale, Renderer,
        ShellKeyAction, ShellKeyEvent, ShortcutKey, Window, WindowDecorationMode,
        WindowFrameFactory, WindowFrameTemplate,
    };
    pub use crate::host::application::{
        Capture, CaptureProtocols, CaptureSources, DirectCaptureAccess, InternalCapture,
        PortalCapture, PortalSessionIntegration, WaylandCapture,
    };
    #[cfg(all(feature = "shell-screencast-linux", target_os = "linux"))]
    pub use crate::portal::*;
    pub use crate::services::session;
    pub use crate::services::session::{GuiSessionConfig, ApplicationRegistry, ApplicationSpec, ApplicationRef, ApplicationHandle, SessionApplications};
    pub use crate::{ClientCursorMode, CursorGraphic, CursorTheme, cursor};
    pub use crate::{DecorationNegotiation, DecorationPolicy};
}
#[cfg(feature = "application-software")]
pub use crate::host::application::HeadlessRuntime;
#[cfg(any(
    feature = "application-software",
    any(all(feature = "application-vulkan-windows", target_os = "windows"), all(feature = "application-vulkan-linux", target_os = "linux")),
    all(feature = "shell-wayland-linux", target_os = "linux")
))]
pub use crate::host::application::{
    AppError, AppResult, AppRuntime, Application, ComposedAppRuntime, Compositor, FrameDiagnostics,
    GuiApplication, ManagedComponentRuntime, ManagedComponentTaskTurn, ManagedTaskCapabilities,
    ManagedTaskDiagnostics, ManagedTaskExecutor, ManagedTaskHost, ManagedTaskPoll, PlatformInput,
    PreparedFrame, ReadyCompositor, ReadyGuiApplication, ReadyShellEnvironment, ReadyWindow,
    Renderer, SceneDeltaQueue, ShellEnvironment, ShellEnvironmentWithCompositor, Window,
    WindowDecorationMode, WindowFrameFactory, WindowFrameTemplate, WindowOptions,
};
pub type Result<T> = crate::host::application::AppResult<T>;
pub use crate::authoring::compose::{
    HoverEffect, Alignment, Component, ComponentFields, ComponentInstanceId, Dimension, EasyWindowFrame,
    Element, EventContext, EventHandler, InputsChangedContext, Insets, Key, MountContext,
    PointerViewExt, RuntimeTarget, Signal, SignalSnapshot, SignalWriter, TextStyle,
    UnmountContext as CompositionUnmountContext, View, ViewError, WindowChromeDesign,
    WindowChromeDesignError, WindowChromePalette, WindowChromeStateStyle, WindowControlButtonStyle,
    WindowControlDesign, WindowControlVisual, WindowControlsDesign, WindowTitleBarStyle, button,
    card, checkbox, column, easy_window_frame, hashed_key, image, row, slider, spacer, stack,
    switch, text, window_content_slot, window_frame,
};
pub use crate::components::application::primitives::{
    ApplicationPrimitiveDiagnosticCollector, ApplicationPrimitiveDiagnosticKind,
    ApplicationPrimitiveDiagnostics, ApplicationRegion, ApplicationRegionError,
    ApplicationRegionKind, ApplicationRegionRef, ApplicationRegionStyle, ApplicationRoot,
    ApplicationRootError, ApplicationRootRef, ApplicationRootStyle, ApplicationUiExt,
    AxisConstraints, ColorSchemePreference, EnvironmentChangeSet, EnvironmentDiagnostics,
    EnvironmentError, EnvironmentGeometryAspect, EnvironmentInputAspect,
    EnvironmentLanguageAndDirectionAspect, EnvironmentPreferences, EnvironmentPreferencesAspect,
    EnvironmentReadBinding, EnvironmentReads, EnvironmentRevision,
    EnvironmentScaleAndDensityAspect, EnvironmentSnapshot, EnvironmentState, EnvironmentUpdate,
    EnvironmentValues, EnvironmentViewAspect, EnvironmentViewState, HudCoordinateSpace,
    HudHitTestPolicy, HudLayer, HudLayerError, HudLayerRef, HudLayerStyle, HudSemanticPolicy,
    InputCapabilities, LocaleTag, LogicalConstraints, LogicalDensityClass, PreferredReadingOrder,
    RenderTargetToken, RenderTargetView, RenderTargetViewContent, RenderTargetViewError,
    RenderTargetViewRef, RenderTargetViewSemanticPolicy, RenderTargetViewStyle, VideoColorMetadata,
    VideoColorPrimaries, VideoColorRange, VideoFit, VideoProtection, VideoSurface,
    VideoSurfaceContent, VideoSurfaceError, VideoSurfaceRef, VideoSurfaceSemanticPolicy,
    VideoSurfaceStyle, VideoSurfaceToken, VideoTransferFunction, ViewportOverlay,
    ViewportOverlayPlacement, ViewportOverlayPlacementError, ViewportOverlayRef,
    ViewportOverlayStyle, WorldAnchor, WorldAnchorProjection, WorldAnchorProjectionError,
    WorldAnchorRef, WorldAnchorStyle, WorldAnchorVisibility,
};
pub use crate::components::application::{
    ActionFactory, ActivityIndicator, ActivityIndicatorDensityStyle, ActivityIndicatorError,
    ActivityIndicatorRef, ActivityIndicatorState, ActivityIndicatorStyle,
    ActivityIndicatorVisualStyle, ActivityMotionPreference, ActivityMotionStyle, AdaptiveScaffold,
    AdaptiveScaffoldError, AdaptiveScaffoldPlan, AdaptiveScaffoldPolicy,
    AdaptiveScaffoldPolicyError, AdaptiveScaffoldRef, AdaptiveScaffoldStyle,
    AdaptiveScaffoldTransition, AdaptiveSlotPlan, AdaptiveSlotPresentation, AdaptiveSlotTransition,
    AdaptiveWidthClass, ApplicationOverlayCommand, ApplicationOverlayController,
    ApplicationOverlayControllerError, ApplicationOverlayControllerState, ApplicationOverlayEffect,
    ApplicationOverlayHost, ApplicationOverlayHostError, ApplicationOverlayHostRef,
    ApplicationPopupPlacement, ApplicationPopupPlacementError, ApplicationPopupPlacementPolicy,
    ApplicationPopupPlacementRequest, Breadcrumb, BreadcrumbError, BreadcrumbItem,
    BreadcrumbItemError, BreadcrumbItemRef, BreadcrumbRef, BreadcrumbSelectionRequest,
    BreadcrumbStyle, Button, ButtonBehavior, ButtonBusyPolicy, ButtonError, ButtonInteractionState,
    ButtonRef, ButtonStyle, ButtonStyleState, ButtonVisualStyle, ChangePhase, CheckCycleError,
    CheckCyclePolicy, CheckState, Checkbox, CheckboxError, CheckboxRef, CheckboxStateStyle,
    CheckboxStyle, CheckboxVisualStyle, CommandInvocation, CommandModelError, CommandOwnerField,
    CommandShortcut, CommandShortcutOutcome, CommandShortcutRegistration,
    CommandShortcutRegistrationError, CommandShortcutScope, CommandShortcutScopeError, CommandSpec,
    CompositionChanged, ContextMenu, ContextMenuDismissal, ContextMenuError,
    ContextMenuOpenRequest, ContextMenuOpened, ContextMenuOpening, DataGrid, DataGridActivation,
    DataGridCell, DataGridDiagnostics, DataGridError, DataGridNavigation, DataGridRef,
    DataGridStyle, DensityClass, DensityError, DensityMetrics, Dialog, DialogBarrierIntent,
    DialogBarrierPolicy, DialogError, DialogInitialFocus, DialogKind, DialogOpened, EditHistory,
    EditHistoryAvailability, EditHistoryCommand, EditHistoryDiagnostics, EditHistoryError,
    EditHistoryKind, EditHistoryPolicy, EditHistoryPolicyError, EditHistoryRecordOutcome,
    EditRejected, EditRejectedReason, FieldMetadata, FieldMetadataError, FieldSemanticSupport,
    FieldValidation, Form, FormAcceptedSubmission, FormDiagnostics, FormError, FormFocusIntent,
    FormInvalidSubmission, FormRevealIntent, FormSubmission, FormUpdate, IconArtwork, IconButton,
    IconButtonError, IconButtonRef, IconButtonStyle, IconButtonVisualStyle, IconSlotStyle,
    IconSlotStyleError, ImageView, ImageViewContent, ImageViewError, ImageViewRef,
    ImageViewSemanticPolicy, ImageViewStyle, InteractiveTargetSize, Label, LabelContent,
    LabelError, LabelRef, LabelStyle, LabelTextStyle, LabelTextStyleError, Link, LinkAction,
    LinkCommand, LinkCommandKind, LinkDestination, LinkDestinationError, LinkError, LinkRef,
    LinkStyle, LinkVisualStyle, ListBox, ListBoxDiagnostics, ListBoxError, ListBoxItemsUpdate,
    ListBoxOption, ListBoxOptionError, ListBoxOptionRef, ListBoxRef, ListBoxSelectionRequest,
    ListBoxStyle, ListBoxTransition, ListView, ListViewDiagnostics, ListViewError, ListViewItem,
    ListViewItemError, ListViewMove, ListViewRef, ListViewRowRef, ListViewStyle, ListViewUpdate,
    Menu, MenuActivationDismissal, MenuButton, MenuButtonError, MenuButtonOpenRequest,
    MenuButtonRef, MenuCommandIntent, MenuController, MenuControllerError, MenuDispatch, MenuError,
    MenuInteractionError, MenuItem, MenuItemKind, MenuItemRef, MenuLevelState, MenuNavigation,
    MenuOpenRequest, MenuOpened, MenuOpeningFocus, MenuRef, MenuRouteRequest, MenuStyle,
    MenuSubmenuCancellation, MenuSubmenuDeadline, MenuSubmenuIntent, MenuTypeaheadIntent, Meter,
    MeterBand, MeterBands, MeterError, MeterLevel, MeterLevelColors, MeterRef, MeterStyle,
    MeterVisualStyle, NavigationBar, NavigationBarBehavior, NavigationBarDestination,
    NavigationBarDestinationError, NavigationBarDestinationRef, NavigationBarError,
    NavigationBarNavigation, NavigationBarNavigationKind, NavigationBarPolicy, NavigationBarRef,
    NavigationBarSelectionRequest, NavigationBarStyle, NavigationController, NavigationDiagnostics,
    NavigationEntry, NavigationError, NavigationRail, NavigationRailBehavior,
    NavigationRailDestination, NavigationRailDestinationError, NavigationRailDestinationRef,
    NavigationRailError, NavigationRailNavigation, NavigationRailNavigationKind,
    NavigationRailPolicy, NavigationRailRef, NavigationRailSelectionRequest, NavigationRailStyle,
    NavigationRestorationKey, NavigationSelectionRequest, NavigationTransition,
    NavigationTransitionKind, NumericCommit, NumericField, NumericFieldCommand,
    NumericFieldCommandAvailability, NumericFieldError, NumericFieldOutput, NumericFieldRef,
    NumericFieldScalar, NumericFieldState, NumericIntermediate, NumericInvalid,
    NumericScalarParseError, Popup, PopupAnchor, PopupError, PopupOpened, ProgressDensityStyle,
    ProgressError, ProgressIndicator, ProgressMode, ProgressRef, ProgressStyle, ProgressValue,
    ProgressVisualStyle, RadioGroup, RadioGroupBehavior, RadioGroupError, RadioGroupRef,
    RadioGroupTransition, RadioItem, RadioItemError, RadioItemRef, RadioItemStateStyle,
    RadioItemVisualStyle, RadioStyle, RangeAffix, RangeFormat, RangeMark, RangeModel,
    RangeModelError, RangeNumber, RangeScalar, ResolvedActivityIndicatorStyle, ResolvedButtonStyle,
    ResolvedCheckboxStyle, ResolvedCommandShortcut, ResolvedCommandState, ResolvedIconButtonStyle,
    ResolvedLinkStyle, ResolvedMeterStyle, ResolvedProgressStyle, ResolvedRadioItemStyle,
    ResolvedSheetEdge, ResolvedSliderStyle, ResolvedSwitchStyle, ResolvedToastCorner,
    ResolvedToastExtent, ResolvedToggleButtonStyle, STANDARD_APPLICATION_POPUP_CANDIDATES,
    Scaffold, ScaffoldError, ScaffoldRef, ScaffoldSlot, ScaffoldSlotRef, ScaffoldSlotSpec,
    ScaffoldSlotSpecError, ScaffoldStyle, ScrollBar, ScrollBarBehavior, ScrollBarCommand,
    ScrollBarError, ScrollBarModel, ScrollBarRef, ScrollBarStyle, ScrollBarThumbGeometry,
    ScrollBarTrackGeometry, ScrollController, ScrollControllerCommand, ScrollControllerError,
    ScrollControllerOutcome, ScrollView, ScrollViewAxis, ScrollViewBehavior, ScrollViewCommand,
    ScrollViewError, ScrollViewRef, ScrollViewStyle, SearchField, SearchFieldCommand,
    SearchFieldCommandAvailability, SearchFieldError, SearchFieldOutput, SearchFieldRef,
    SecureContentExposure, SecureContextCapabilities, SecureContextCommandAvailability,
    SecureField, SecureFieldCommand, SecureFieldCommandAvailability, SecureFieldError,
    SecureFieldOutput, SecureFieldPrivacyPolicy, SecureFieldRef, SecureFieldUpdate, SelectableText,
    SelectableTextBehavior, SelectableTextError, SelectableTextRef, SelectionChanged,
    SelectionDiagnostics, SelectionError, SelectionFollowsFocus, SelectionItemsUpdate,
    SelectionMode, SelectionModel, SelectionProposal, SelectionProposalKind, SelectionTransition,
    Separator, SeparatorError, SeparatorGeometry, SeparatorOrientation, SeparatorRef,
    SeparatorSemanticPolicy, SeparatorStyle, Sheet, SheetBarrierIntent, SheetBarrierPolicy,
    SheetEdge, SheetError, SheetExtent, SheetInitialFocus, SheetMode, SheetOpened,
    ShortcutDisplayBinding, ShortcutDisplayBindingError, ShortcutSet, ShortcutSetError, Slider,
    SliderBehavior, SliderCommand, SliderError, SliderInteractionState, SliderOrientation,
    SliderPointerOutcome, SliderRef, SliderStyle, SliderStyleState, SliderTrackGeometry,
    SliderVisualStyle, Submitted, Switch, SwitchError, SwitchRef, SwitchStateStyle, SwitchStyle,
    SwitchVisualStyle, Tab, TabActivationPolicy, TabBehavior, TabError, TabNavigation,
    TabNavigationKind, TabOrientation, TabPanelRef, TabPolicy, TabRef, TabSelectionRequest, Table,
    TableCell, TableCellRef, TableColumn, TableColumnError, TableColumnRef, TableError, TableRef,
    TableRow, TableRowError, TableRowRef, TableStyle, Tabs, TabsError, TabsRef, TabsStyle,
    TargetAssessment, TextArea, TextAreaCommand, TextAreaError, TextAreaOutput, TextAreaRef,
    TextAreaReturnPolicy, TextChanged, TextController, TextControllerError,
    TextControllerHistoryError, TextControllerSessionOutcome, TextControllerUpdate, TextField,
    TextFieldCommand, TextFieldCommandAvailability, TextFieldError, TextFieldMode, TextFieldOutput,
    TextFieldRef, TextFieldStyle, TextFieldVisualStyle, Toast, ToastAnnouncementIntent,
    ToastAnnouncementPolicy, ToastAnnouncementPriority, ToastCoalescingIntent, ToastCoalescingKey,
    ToastCorner, ToastDeadlineError, ToastDismissalIntent, ToastDismissalPolicy, ToastError,
    ToastExpiryIntent, ToastExtent, ToastLifetime, ToastLifetimeError, ToastOpened,
    ToastRedactionIntent, ToggleButton, ToggleButtonError, ToggleButtonRef, ToggleButtonStyle,
    Toolbar, ToolbarBehavior, ToolbarCommandRequest, ToolbarError, ToolbarInvocation,
    ToolbarInvocationError, ToolbarItemRef, ToolbarNavigationPolicy, ToolbarOrientation,
    ToolbarRef, ToolbarStyle, ToolbarTransition, Tooltip, TooltipAccessibleContribution,
    TooltipAnchor, TooltipDeadlineError, TooltipDeadlineIntent, TooltipDismissalPolicy,
    TooltipError, TooltipExtent, TooltipOpened, TooltipSemanticsIntent, TooltipTrigger,
    TooltipTriggerPolicy, TooltipTriggerPolicyError, TreeExpansionProposal,
    TreeExpansionTransition, TreeGrid, TreeGridActivation, TreeGridDiagnostics, TreeGridError,
    TreeGridNavigation, TreeGridRef, TreeHierarchy, TreeHierarchyDiagnostics, TreeHierarchyError,
    TreeItem, TreeItemError, TreeItemRef, TreeView, TreeViewActivation, TreeViewDiagnostics,
    TreeViewError, TreeViewExpansionTransition, TreeViewNavigation, TreeViewRef, TreeViewStyle,
    ValidationKind, ValidationMessage, ValidationResult, ValidationResultError, ValidationSummary,
    ValidationSummaryAction, ValidationSummaryEntry, ValidationSummaryEntryRef,
    ValidationSummaryError, ValidationSummaryRef, ValidationSummaryStyle, ValueChange,
    VirtualListError, VirtualListPlan, VirtualListPolicy, VirtualListPolicyError,
    VirtualListRowRef, VirtualListStyle, VirtualListTotal, VirtualListUpdate, VirtualListView,
    VirtualListViewRef, VirtualListViewport, VirtualListViewportError, place_application_popup,
    standard_listbox_policy, standard_radio_policy,
};
pub use crate::components::shell::*;
pub use crate::foundation::{
    ColorRgba8, EdgeInsets, PointF, PointI, RectF, RectI, SizeF, SizeI, Transform2D,
};
pub use crate::graphics::material::{
    MaterialContract, MaterialLibrary, MaterialPass, MaterialPassKind,
};
pub use crate::graphics::render::{
    AlphaMode, BatchKey, BlendMode, BoxInstance, ColorSpace, CompileStats, DamageRegion,
    DenseInstances, DirtyRanges, DrawItem, GlyphInstance, ImageInstance, MaterialInstance,
    PipelineKind, PrimitiveKind, RangePatch, ReadbackFormat, ReadbackImage, ReadbackRequest,
    RenderBackend, RenderClip, RenderError, RenderErrorKind, RenderRequest, RenderResult,
    RenderScene, RenderSceneDelta, RenderSpatialNode, RenderStats, RenderTargetInfo, SceneCompiler,
    SceneUpdateStats, TargetLoad, TargetStore,
};
pub use crate::input::{
    Activation, ActivationCancelReason, ActivationInput, ActivationOutcome, ActivationPhase,
    ActivationStateMachine, ActivationTransition, ActiveShortcutScope, ButtonState, ChangeSource,
    CompetingGesture, CompositeChange, CompositeDiagnostics, CompositeEdgeBehavior,
    CompositeEntryReason, CompositeError, CompositeFocusTarget, CompositeHighlightReason,
    CompositeItem, CompositeNavigationCommand, CompositeNavigationPolicy, CompositeOrientation,
    CompositeSelectionBehavior, CompositeSelectionRequest, CompositeStateMachine, DefaultResponse,
    DisabledItemPolicy, DragAxis, DragRecognizer, EventPhase, FocusCandidate, FocusChange,
    FocusClearReason, FocusDiagnostics, FocusError, FocusIndicatorPolicy, FocusInputModality,
    FocusMoveReason, FocusOrigin, FocusScopeId, FocusStateMachine, FocusTraversalDirection,
    FocusTraversalEdge, GestureArena, GestureArenaDecision, GestureArenaDiagnostics,
    GestureArenaError, GestureArenaLossReason, GestureArenaRequest, GestureArenaWinReason,
    GestureCancelReason, GestureDeadlineId, GestureDeadlineRequest, GestureDelta, GestureInput,
    GestureKind, GestureOutcome, GestureRecognizerDiagnostics, GestureRecognizerError,
    GestureRecognizerState, GestureTransition, InputEvent, KeyEvent, KeyLocation, KeyText,
    KeyTextError, LogicalKey, LongPressRecognizer, MAX_KEY_TEXT_BYTES, MAX_PRESSED_POINTER_BUTTONS,
    Modifiers, NamedKey, PhysicalKey, PhysicalKeyCode, PhysicalPointerPosition,
    PhysicalScrollDelta, PointerButton, PointerButtonSet, PointerButtonSetError,
    PointerCancelReason, PointerCaptureChange, PointerCaptureRequest, PointerContactGeometry,
    PointerCoordinateError, PointerDeviceId, PointerDeviceKind, PointerEvent, PointerEventError,
    PointerEventKind, PointerEventSource, PointerId, PointerInputEvent, PointerPosition,
    PointerPressure, PointerProperties, PointerPropertyError, PointerStateSnapshot, PointerTilt,
    PointerTwist, Propagation, ScrollDelta, ScrollEvent, ScrollMomentumPhase, ScrollPhase,
    ScrollPrecision, ScrollUnit, ScrollValueError, ShortcutBinding, ShortcutChord,
    ShortcutDiagnostics, ShortcutError, ShortcutMatcher, ShortcutRepeatPolicy, ShortcutResolution,
    ShortcutScopeId, ShortcutScopePolicy, ShortcutTrigger, TapRecognizer, ValueChangePhase,
    WritingDirection,
};
pub use crate::platform::contracts::{
    AccessibilityActionAdmission, AccessibilityActionAdmissionError, AccessibilityActionEvent,
    AccessibilityAdmissionError, AccessibilityApplied, AccessibilityCapability,
    AccessibilityCapabilityQuery, AccessibilityLimitError, AccessibilityLimits,
    AccessibilityOperations, AccessibilityPublicationAdmission, AccessibilityPublicationRequest,
    AccessibilityService, AccessibilityServiceKey, ActivityState, AdmittedRequest, AvoidRegion,
    AvoidRegionKind, CapabilityDescriptor, CapabilityLimit, ClipboardAdmissionError,
    ClipboardCapabilities, ClipboardCapability, ClipboardCapabilityError, ClipboardChange,
    ClipboardClearApplied, ClipboardClearRequest, ClipboardKind, ClipboardLimitError,
    ClipboardLimits, ClipboardOperations, ClipboardPublishApplied, ClipboardPublishRequest,
    ClipboardRequestAdmission, ClipboardRequestError, ClipboardRevision, ClipboardService,
    ClipboardServiceKey, ClipboardSnapshot, ClipboardSnapshotError, ClipboardSnapshotId,
    ClipboardSnapshotStatus, CloseRequest, CloseRequestDecision, CloseRequestReason,
    CoalescingMetadata, CollapsedEventCount, CoordinateSpace, CursorAdmissionError,
    CursorAnimationFrame, CursorAppearance, CursorAppearanceAdmission, CursorAppearanceApplied,
    CursorAppearanceRequest, CursorCapability, CursorCapabilityQuery, CursorConstraintAdmission,
    CursorConstraintKind, CursorConstraintLease, CursorConstraintLeaseHandle,
    CursorConstraintLeaseId, CursorConstraintLeaseStatus, CursorConstraintRequest,
    CursorConstraintRevocation, CursorImageError, CursorLimitError, CursorLimits, CursorOperations,
    CursorPositionAdmission, CursorPositionApplied, CursorPositionError, CursorPositionRequest,
    CursorSelection, CursorSelectionKind, CursorService, CursorServiceKey, CustomCursor,
    CustomCursorAnimation, CustomCursorImage, DataFormat, DataFormatError, DataFormatKind,
    DataFormatReadRequest, DataOfferDescriptor, DataOfferError, DataOfferId, DataReadAdmission,
    DataReadCompletion, DataReadMetadataError, DataReadMode, DataReadProgress,
    DataReadValidationError, DataSourceKind, DataTransferAdmissionError, DataTransferCapability,
    DataTransferLimitError, DataTransferLimits, DataTransferOperations, DataTransferService,
    DataTransferServiceKey, DisplayAccuracy, DisplayAccuracyProfile, DisplayCapability,
    DisplayChange, DisplayChangeError, DisplayColorSpace, DisplayDescriptor,
    DisplayDescriptorError, DisplayId, DisplayLimitError, DisplayLimits, DisplayLogicalBounds,
    DisplayOperations, DisplayOrientation, DisplayProperties, DisplayRevision, DisplayService,
    DisplayServiceKey, DisplaySnapshot, DisplaySnapshotError, DisplaySnapshotStatus,
    DisplayTransform, EventStamp, EventStampError, EventStampStream, ExecutionRequirement,
    ExternalUri, ExternalUriError, FileDialogAdmission, FileDialogAdmissionError,
    FileDialogCapability, FileDialogCapabilityQuery, FileDialogFilter, FileDialogFilterError,
    FileDialogFilterRule, FileDialogLimitError, FileDialogLimits, FileDialogMode,
    FileDialogOperations, FileDialogOptions, FileDialogOptionsError, FileDialogRequest,
    FileDialogResult, FileDialogSelection, FileDialogSelectionError, FileDialogService,
    FileDialogServiceKey, FileExtension, FileExtensionError, ForcedDestruction,
    ForcedDestructionPhase, HAPTIC_INTENSITY_UNITS, HapticAdmission, HapticAdmissionError,
    HapticApplied, HapticCapability, HapticCapabilityError, HapticDeviceSupport,
    HapticDeviceSupportError, HapticEffect, HapticEffectSupport, HapticIntensity,
    HapticIntensityError, HapticLimitError, HapticLimits, HapticOperations, HapticRequest,
    HapticUserSettingState, HapticsService, HapticsServiceKey, HdrState, InsetKind, LifecycleAxis,
    LifecycleError, LifecycleTransition, LogicalToPhysicalTransform, MAX_AVOID_REGIONS,
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
    MAX_REDRAW_VIEWS, MAX_RESTORATION_TOKEN_BYTES, MAX_SELECTED_RESOURCE_NAME_BYTES,
    MAX_SELECTED_RESOURCES, MAX_SUGGESTED_FILE_NAME_BYTES, MAX_TEXT_INPUT_SURROUNDING_BYTES,
    MAX_URI_SCHEME_BYTES, MAX_URI_SCHEMES, MAX_WINDOW_TITLE_BYTES, MenuAccelerator,
    MenuAcceleratorError, MenuAcceleratorLabel, MenuActionAdmission, MenuActionAdmissionError,
    MenuActionEvent, MenuActionRequest, MenuActionSource, MenuAdmissionError, MenuCapability,
    MenuCapabilityQuery, MenuCheckState, MenuItem as PlatformMenuItem, MenuItemError, MenuItemId,
    MenuItemKind as PlatformMenuItemKind, MenuItemState, MenuLabel, MenuLimitError, MenuLimits,
    MenuOperations, MenuPublicationAdmission, MenuPublicationApplied, MenuPublicationError,
    MenuPublicationRequest, MenuRevision, MenuRole, MenuScope, MenuService, MenuServiceKey,
    MenuSnapshotId, MenuTextError, MenuTree, MenuTreeError, MetricInsets, MetricsCitation,
    MetricsRevision, MonotonicClock, MonotonicClockError, MonotonicClockState,
    NativeSurfaceGeneration, NativeSurfaceState, NoCapabilityLimits, NotificationAction,
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
    NotificationSnapshotId, NotificationTextError, NotificationTitle, PendingHostFacts,
    PermissionState, PhysicalExtent, PlatformError, PlatformErrorKind, PlatformErrorSource,
    PlatformEvent, PlatformResult, PointerIcon, PostTurnSchedule, PowerAdmissionError,
    PowerCapability, PowerCapabilityQuery, PowerInhibitionAdmission, PowerInhibitionKind,
    PowerInhibitionLease, PowerInhibitionLeaseHandle, PowerInhibitionLeaseId,
    PowerInhibitionLeaseStatus, PowerInhibitionReason, PowerInhibitionRequest,
    PowerInhibitionRevocation, PowerInhibitionScope, PowerLimitError, PowerLimits, PowerOperations,
    PowerPolicyState, PowerService, PowerServiceKey, RemainingWork, RequestAdmission,
    RequestCompletion, RequestId, RequestOutcome, RestorationAdmissionError, RestorationCapability,
    RestorationCapabilityQuery, RestorationClearAdmission, RestorationClearApplied,
    RestorationClearRequest, RestorationConsumptionAdmission, RestorationConsumptionApplied,
    RestorationConsumptionRequest, RestorationLimitError, RestorationLimits, RestorationOperations,
    RestorationPublicationAdmission, RestorationPublicationApplied, RestorationPublicationError,
    RestorationPublicationRequest, RestorationRecord, RestorationRevision, RestorationScope,
    RestorationService, RestorationServiceKey, RestorationSessionId, RestorationSnapshotId,
    RestorationToken, RestorationTokenError, SandboxAccessGrant, SandboxAccessGrantHandle,
    SandboxAccessPolicy, ScaleFactor, ScheduleError, SelectedResource, SelectedResourceAccess,
    SelectedResourceKind, SelectedResourceName, SelectedResourceNameError, ServiceKey,
    ServiceLookup, ServiceRegistration, ServiceRegistry, ServiceRemoval, ServiceReplacement,
    ServiceUnavailable, SizeHint, StandardCursor, StatusMenuId, SuggestedFileName,
    SuggestedFileNameError, Support, TextInputAdmission, TextInputAdmissionError, TextInputApplied,
    TextInputCapability, TextInputCapabilityQuery, TextInputDeltaEvent, TextInputDeltaKind,
    TextInputLimitError, TextInputLimits, TextInputOperations, TextInputService,
    TextInputServiceKey, TextInputSyncError, TextInputSyncKind, TextInputSyncRequest, TrustLevel,
    UnavailableReason, UriAdmissionError, UriCapabilities, UriCapability, UriCapabilityError,
    UriLimitError, UriLimits, UriOpenAdmission, UriOpenApplied, UriOpenRequest, UriOperation,
    UriScheme, UriSchemeCapability, UriSchemeError, UriService, UriServiceKey, UserGestureGrant,
    UserGestureGrantHandle, UserGestureRequirement, ViewDisplayError, ViewDisplaySnapshot,
    ViewDisplayStatus, ViewId, ViewLifecycle, ViewLifetime, ViewMetrics, ViewMetricsError,
    ViewMetricsSnapshot, ViewMetricsState, ViewMetricsUpdate, ViewRevision, ViewSnapshot,
    ViewState, ViewStateError, ViewUpdate, VisibilityState, WindowAdmissionError,
    WindowAttentionApplied, WindowAttentionIntent, WindowAttentionRequest, WindowCapability,
    WindowCapabilityLimits, WindowCapabilityQuery, WindowCloseApplied, WindowCloseIntent,
    WindowCloseRequest, WindowConstraintAxis, WindowConstraintBound, WindowOperation,
    WindowRequestAdmission, WindowService, WindowServiceKey, WindowSizeConstraints,
    WindowSizeConstraintsApplied, WindowSizeConstraintsError, WindowSizeConstraintsRequest,
    WindowStateApplied, WindowStateIntent, WindowStateRequest, WindowTitle, WindowTitleApplied,
    WindowTitleError, WindowTitleRequest,
};
pub use crate::runtime::{
    Command, Component as MountedComponent, ComponentDiagnostics, ComponentId,
    ComponentRuntimeDriver, CompositionDiagnostics, CompositionDriver, CreateContext,
    FrameScheduler, LifecycleState, LocalTask, LocalTaskSender, MonotonicInstant, NoAction, Read,
    RuntimeError, SendTask, State, SwitchBranch, TaskCancellation, TaskHandle, TaskHost,
    TaskSendError, TaskSender, TimerHandle, Ui, UnmountContext, UnsupportedTaskHost, UpdateContext,
    ViewRuntime,
};
pub use crate::ui::layout::{
    ClipId, ComputedLayout, LayoutDiagnostics, LayoutEngine, MAX_POPUP_OCCLUSIONS,
    PopupOverflowPolicy, PopupPlacement, PopupPlacementAdjustment, PopupPlacementAlignment,
    PopupPlacementCandidate, PopupPlacementError, PopupPlacementRequest, PopupPlacementSide,
    RevealAlignment, RevealRequest, ScrollActivity, ScrollAnchorMode, ScrollCancelReason,
    ScrollChangeSource, ScrollDiagnostics, ScrollError, ScrollExtentAnchor, ScrollInputSource,
    ScrollMetrics, ScrollMotionId, ScrollMotionRequest, ScrollPhysics, ScrollState, ScrollUpdate,
    SpatialId, VirtualCollection, place_popup,
};

pub use crate::graphics::scene::{
    DirtyFlags, NodeArena, NodeCore, NodeId, SparseSet, SubtreeRange,
};
pub use crate::theme::{
    CatalogStyle, CompiledComponentStyle, CompiledSlotStyle, CompiledStateStyle, CompiledTheme,
    ComponentStyleContract, Easing, InteractionState, MotionPreference, ResolvedComponentStyle,
    ShadowSource, SlotStyleSource, StateStyleSource, StylePropertyMask, StyleSlotContract,
    ThemeCatalog, ThemeDiagnostic, ThemeDomain, ThemeError, ThemeFormat, ThemeReplacement,
    ThemeResult, ThemeRuntime, ThemeRuntimeDiagnostics, ThemeScope, ThemeScopeKind, ThemeSource,
    ThemeTokensSource, ThemeUpdate, TransitionSource, TransitionSpec, TypographySource,
    ValueSource, VariantStyleSource, foundation_catalog, validate_archive_header,
};
pub use crate::ui::text::{
    AtlasGlyph, AtlasPageUpdate, GlyphAtlas, GlyphAtlasView, PreparedText, ResolvedTextStyle,
    RetainedTextRequest, RetainedTextRun, RetainedTextSystem, TEXT_SEGMENTATION_CRATE_VERSION,
    TEXT_SEGMENTATION_PROFILE, TEXT_SEGMENTATION_UNICODE_VERSION, TextAffinity, TextBuffer,
    TextBufferError, TextCacheStats, TextChange, TextChunk, TextChunks, TextCompositionCommand,
    TextCompositionError, TextCompositionKind, TextEdit, TextEditBatch, TextEditError,
    TextEditOutcome, TextEngine, TextError, TextInputConfiguration, TextInputGeometry,
    TextInputPolicy, TextInputPurpose, TextInputRequest, TextInputResyncReason, TextInputSession,
    TextInputSnapshot, TextLayoutRequest, TextMultiline, TextNavigationDirection,
    TextNavigationUnit, TextOffset, TextRange, TextRangeError, TextResult, TextReturnKeyAction,
    TextRevision, TextRunId, TextRunKey, TextSelection, TextSelectionAdjustment,
    TextSessionCommand, TextSessionDelta, TextSessionDeltaOutcome, TextSessionId, TextSessionPhase,
    TextSessionStateError, TextSnapshot, TextSurroundingText, TextVirtualKeyboardPreference,
};
pub use crate::ui::{
    Background, Border, BorderSide, BoxDecoration, BoxDecorationError, BoxSizing, BoxStyle,
    ComponentStyleId, ControlBehavior, ControlHandle, CornerRadii, CrossAxisAlignment,
    DismissReason, Flow, ImageId, ImageVisual, InteractionFlags, InteractionSnapshot, LayoutStyle,
    MainAxisAlignment, MaterialId, MountWriter, MountedUi, NodeKind, Outline, OutsidePressPolicy,
    Overflow, OverlayAnchor, OverlayCloseOutcome, OverlayDiagnostics, OverlayDismissPolicy,
    OverlayDismissResult, OverlayDismissed, OverlayEntry, OverlayError, OverlayFocusContainment,
    OverlayFocusLifecycle, OverlayFocusRequest, OverlayFocusRestoration, OverlayHost, OverlayId,
    OverlayInitialFocus, OverlayModality, OverlayOpenRequest, OverlayOpened, Property,
    SemanticAction, SemanticActions, SemanticCheckState, SemanticCollection, SemanticError,
    SemanticName, SemanticNode, SemanticParticipation, SemanticRelationship,
    SemanticRelationshipKind, SemanticRole, SemanticState, SemanticValue, Shadow, ShadowList,
    SizeRule, SizeRule2D, StringId, StyleBinding, StyleId, StyleOverride, StylePropertyPatch,
    StyleSlotBinding, StyleSlotId, StyleVariantSelection, TextAlign, TextHandle, TextVisual,
    ThemeDomainId, ThemeScopeId, TransactionResult, UiDiagnostics, UiEvent, UiEventKind,
    UiMemoryReport, UiRoot, UiTransaction, VariantAxisId, VariantValueId,
};

pub use crate::authoring::compose::{
    ShellAttachment, ShellChild, ShellDismissReason, ShellEdge, ShellExtent, ShellFocus,
    ShellOutputPreview, ShellPlacementBounds, ShellPointer, ShellReservation, ShellSurfaceLayer,
    ShellSurfaceSpec, ShellWidget, ShellWindowPreview, WidgetPlacement,
};

pub use crate::authoring::compose::{
    ApplicationCatalog, ApplicationCatalogHandle, ApplicationCatalogStatus, ApplicationIcon,
    ApplicationId, ApplicationMetadata, ApplicationQuery, ApplicationVisibility, IconRequest,
    ShellContext, ShellRequestCompletion, ShellRequestOutcome, ShellServiceError, ShellServices,
    ShellWindow, ShellWindowAction, ShellWindows,
};

pub use crate::authoring::compose::{
    TilePreviewDesign, TilePreviewMotion, TileTarget, WindowTiling,
};

mod compat;
pub use compat::*;

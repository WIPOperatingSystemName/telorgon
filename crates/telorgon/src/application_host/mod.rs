//! Mounted application lifecycle, renderer-free frame preparation, and managed host assembly.

#[cfg(all(feature = "shell-wayland-linux", not(target_os = "linux")))]
compile_error!("feature `shell-wayland-linux` is supported only for Linux targets");

mod declaration;
mod capture_config;
pub use capture_config::{Capture, CaptureSources, PortalCapture, PortalSessionIntegration,
    WaylandCapture, CaptureProtocols, DirectCaptureAccess, InternalCapture};
mod output_scale;
pub use output_scale::OutputScale;
mod delta_queue;
#[cfg(all(feature = "shell-wayland-linux", target_os = "linux"))]
mod shell_wayland;
// Keep the platform-neutral compositor transaction and retained-scene tests executable on the
// development host even when the native Wayland/KMS owner is compiled only for Linux.
mod error;
mod exit;
#[cfg(feature = "application-software")]
mod headless;
mod input;
mod interaction;
mod keybindings;
#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows")
))]
mod native;
#[cfg(any(
    feature = "application-software",
    all(feature = "application-vulkan-windows", target_os = "windows"),
    all(feature = "shell-wayland-linux", target_os = "linux")
))]
mod profiler;
mod runtime;
mod scheduler;
#[cfg(test)]
#[path = "shell_wayland/backend_boundary.rs"]
mod shell_wayland_backend_boundary_tests;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "shell_wayland/scene.rs"]
mod shell_wayland_scene_tests;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "shell_wayland/state.rs"]
mod shell_wayland_state_tests;
#[cfg(all(test, not(target_os = "linux"), feature = "application-software"))]
#[path = "shell_wayland/transparency_tests.rs"]
mod shell_wayland_transparency_tests;
mod task_host;
#[cfg(all(test, not(feature = "shell-wayland-linux")))]
#[path = "shell_wayland/capture.rs"]
mod capture_tests;
mod window;

pub use crate::input::{
    ButtonState, DefaultResponse, EventPhase, InputEvent, KeyEvent, KeyLocation, KeyText,
    KeyTextError, LogicalKey, MAX_KEY_TEXT_BYTES, MAX_PRESSED_POINTER_BUTTONS, Modifiers, NamedKey,
    PhysicalKey, PhysicalKeyCode, PhysicalPointerPosition, PhysicalScrollDelta, PointerButton,
    PointerButtonSet, PointerButtonSetError, PointerCancelReason, PointerCaptureChange,
    PointerContactGeometry, PointerCoordinateError, PointerDeviceId, PointerDeviceKind,
    PointerEvent, PointerEventError, PointerEventKind, PointerEventSource, PointerId,
    PointerInputEvent, PointerPosition, PointerPressure, PointerProperties, PointerPropertyError,
    PointerStateSnapshot, PointerTilt, PointerTwist, Propagation, ScrollDelta, ScrollEvent,
    ScrollMomentumPhase, ScrollPhase, ScrollPrecision, ScrollUnit, ScrollValueError,
};
pub use crate::runtime::{
    Command, Component, ComponentDiagnostics, ComponentDriver, ComponentId, ComponentRuntimeDriver,
    CompositionDiagnostics, CompositionDriver, CreateContext, FrameScheduler, LifecycleState,
    MonotonicInstant, NoAction, Read, RuntimeError, State, SwitchBranch, TimerHandle, Ui,
    UnmountContext, UpdateContext, ViewRuntime,
};
pub use declaration::{
    Application, Compositor, CompositorVisual, GuiApplication, KeyboardConfig, LinuxShellConfig,
    ReadyCompositor, ReadyGuiApplication, ReadyShellEnvironment, ReadyWindow, Renderer,
    ShellEnvironment, ShellEnvironmentWithCompositor, ShellKeyAction, ShellKeyEvent, Window,
    WindowFrameFactory, WindowFrameTemplate,
};
pub use delta_queue::SceneDeltaQueue;
pub use error::{AppError, AppResult};
pub use exit::request_exit;
#[cfg(feature = "application-software")]
pub use headless::HeadlessRuntime;
pub use input::{LISTEN_ACTION, LISTEN_FOCUS, LISTEN_KEY, LISTEN_POINTER, PlatformInput};
pub use interaction::{InteractionDiagnostics, InteractionRouter};
pub use keybindings::{KeyBindings, KeyChord, ShortcutKey};
pub use runtime::{
    AppRuntime, AppRuntimeCore, ComposedAppRuntime, InputFlushOutcome, PreparedFrame,
};
pub use scheduler::FrameDiagnostics;
pub use task_host::{
    ManagedComponentRuntime, ManagedComponentTaskTurn, ManagedTaskCapabilities,
    ManagedTaskDiagnostics, ManagedTaskExecutor, ManagedTaskHost, ManagedTaskPoll,
};
pub use window::{WindowDecorationMode, WindowOptions};

pub use crate::compose::{
    ShellAttachment, ShellChild, ShellDismissReason, ShellEdge, ShellExtent, ShellFocus,
    ShellPlacementBounds, ShellPointer, ShellReservation, ShellSurfaceLayer, ShellSurfaceSpec,
    ShellWidget, WidgetPlacement,
};

pub use crate::compose::{
    ApplicationCatalog, ApplicationCatalogHandle, ApplicationCatalogStatus, ApplicationIcon,
    ApplicationId, ApplicationMetadata, ApplicationQuery, ApplicationVisibility, IconRequest,
    ShellContext, ShellRequestCompletion, ShellRequestOutcome, ShellServiceError, ShellServices,
    ShellWindow, ShellWindowAction, ShellWindows,
};

pub use crate::compose::{TilePreviewDesign, TilePreviewMotion, TileTarget, WindowTiling};

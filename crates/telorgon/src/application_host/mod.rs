//! Mounted application lifecycle, renderer-free frame preparation, and managed host assembly.

#[cfg(all(feature = "desktop-wayland-linux", not(target_os = "linux")))]
compile_error!("feature `desktop-wayland-linux` is supported only for Linux targets");

mod declaration;
mod output_scale;
pub use output_scale::OutputScale;
mod delta_queue;
#[cfg(all(feature = "desktop-wayland-linux", target_os = "linux"))]
mod desktop_wayland;
// Keep the platform-neutral compositor transaction and retained-scene tests executable on the
// development host even when the native Wayland/KMS owner is compiled only for Linux.
#[cfg(test)]
#[path = "desktop_wayland/backend_boundary.rs"]
mod desktop_wayland_backend_boundary_tests;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "desktop_wayland/scene.rs"]
mod desktop_wayland_scene_tests;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "desktop_wayland/state.rs"]
mod desktop_wayland_state_tests;
#[cfg(all(test, not(target_os = "linux"), feature = "application-software"))]
#[path = "desktop_wayland/transparency_tests.rs"]
mod desktop_wayland_transparency_tests;
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
    all(feature = "desktop-wayland-linux", target_os = "linux")
))]
mod profiler;
mod runtime;
mod scheduler;
mod task_host;
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
    Application, Compositor, CompositorVisual, DesktopEnvironment,
    DesktopEnvironmentWithCompositor, DesktopKeyAction, DesktopKeyEvent, GuiApplication,
    KeyboardConfig, LinuxDesktopConfig, ReadyCompositor, ReadyDesktopEnvironment,
    ReadyGuiApplication, ReadyShellWidget, ReadyWindow, Renderer, ShellWidget, ShellWidgetAnchor,
    ShellWidgetExtent, Window, WindowFrameFactory, WindowFrameTemplate,
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

mod callbacks;
mod commit;
mod compositor;
mod configure;
mod data_transfer;
mod decoration;
mod destruction;
mod dispatch;
mod dmabuf;
mod drag;
mod keyboard;
mod outputs;
mod pointer;
mod session_lock;
mod surface_extensions;
mod surfaces;
mod touch;
mod xdg_shell;
use callbacks::{bind_global, destroy_resource, dispatch_resource};
mod shm;
pub use shm::{ShmBufferReader, ShmImage, ShmImageRegion};
mod geometry;
mod input;
#[cfg(test)]
mod wire_tests;
use geometry::*;

mod capture;
mod foreign_toplevel;
pub(crate) use capture::{DirectCaptureCompletion, DirectCaptureJob};
mod capture_buffer;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{c_long, c_void};
use std::fmt;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::foundation::{PointI, RectI};
use crate::integrations::wayland::server::ffi;
use crate::integrations::wayland::server::{
    ClientRef, Display, Global, IncomingRequest, NativeProtocol, OwnedClient, ResourceRef,
};

use crate::integrations::wayland::compositor::synchronization::{
    take_surface_commits_through, take_surface_feedbacks_through,
};
use crate::integrations::wayland::compositor::{
    BufferAttachment, BufferDescriptor, BufferTransform, ClientId, ClientLimits, CompositorAction,
    CompositorCore, ObjectMetadata, ProtocolObjectId, ProtocolObjectKind, Region, ShmBuffer,
    ShmFormat, ShmPool, SurfaceRole, WaylandBufferId, WaylandSurfaceId, XdgConfigure,
    XdgToplevelState,
};

mod dmabuf_feedback;
use dmabuf_feedback::DmaBufFeedback;

const IMPLEMENTED_GLOBALS: &[(&str, ResourceKind, u32)] = &[
    ("zxdg_output_manager_v1", ResourceKind::XdgOutputManager, 3),
    ("wl_compositor", ResourceKind::Compositor, 6),
    ("wl_shm", ResourceKind::Shm, 1),
    ("wl_subcompositor", ResourceKind::Subcompositor, 1),
    ("wl_data_device_manager", ResourceKind::DataDeviceManager, 3),
    ("xdg_wm_base", ResourceKind::XdgWmBase, 7),
    (
        "zxdg_decoration_manager_v1",
        ResourceKind::DecorationManager,
        1,
    ),
    (
        "wp_cursor_shape_manager_v1",
        ResourceKind::CursorShapeManager,
        1,
    ),
    (
        "xdg_toplevel_icon_manager_v1",
        ResourceKind::ToplevelIconManager,
        1,
    ),
    (
        "wp_fractional_scale_manager_v1",
        ResourceKind::FractionalScaleManager,
        1,
    ),
    ("wp_viewporter", ResourceKind::Viewporter, 1),
    ("wp_presentation", ResourceKind::Presentation, 2),
    ("xdg_activation_v1", ResourceKind::Activation, 1),
    (
        "ext_session_lock_manager_v1",
        ResourceKind::SessionLockManager,
        1,
    ),
    (
        "zwp_relative_pointer_manager_v1",
        ResourceKind::RelativePointerManager,
        1,
    ),
    (
        "zwp_keyboard_shortcuts_inhibit_manager_v1",
        ResourceKind::ShortcutInhibitManager,
        1,
    ),
    (
        "zwp_idle_inhibit_manager_v1",
        ResourceKind::IdleInhibitManager,
        1,
    ),
    (
        "zwp_pointer_constraints_v1",
        ResourceKind::PointerConstraints,
        1,
    ),
];

#[derive(Clone, Copy, Debug)]
enum ResourceKind {
    ForeignToplevelList,
    ForeignToplevelHandle,
    ImageCopyCaptureManager,
    ImageCopyCaptureSession(ProtocolObjectId),
    ImageCopyCaptureFrame(ProtocolObjectId),
    OutputCaptureSourceManager,
    OutputCaptureSource(u32),
    Compositor,
    Surface(WaylandSurfaceId),
    Region(ProtocolObjectId),
    Shm,
    ShmPool(ProtocolObjectId),
    Buffer(WaylandBufferId),
    Subcompositor,
    Subsurface(WaylandSurfaceId),
    XdgWmBase,
    XdgPositioner(ProtocolObjectId),
    XdgSurface(WaylandSurfaceId),
    XdgToplevel(WaylandSurfaceId),
    XdgPopup(WaylandSurfaceId),
    XwaylandShell,
    XwaylandSurface(WaylandSurfaceId),
    Callback(WaylandSurfaceId),
    Output(u32),
    Seat(u32),
    Pointer(u32),
    Keyboard(u32),
    Touch(u32),
    DataDeviceManager,
    DataDevice(u32),
    DataSource(ProtocolObjectId),
    DataOffer(ProtocolObjectId),
    LinuxDmaBuf,
    LinuxDmaBufFeedback,
    LinuxBufferParams(ProtocolObjectId),
    DecorationManager,
    ToplevelDecoration(WaylandSurfaceId),
    CursorShapeManager,
    CursorShapeDevice(u32),
    ToplevelIconManager,
    ToplevelIcon(ProtocolObjectId),
    FractionalScaleManager,
    FractionalScale,
    Viewporter,
    Viewport(WaylandSurfaceId),
    Presentation,
    PresentationFeedback(WaylandSurfaceId),
    Activation,
    ActivationToken(ProtocolObjectId),
    SessionLockManager,
    SessionLock(ProtocolObjectId),
    SessionLockSurface(WaylandSurfaceId),
    RelativePointerManager,
    RelativePointer(u32),
    XdgOutputManager,
    XdgOutput(u32, ProtocolObjectId),
    XwaylandKeyboardGrabManager,
    XwaylandKeyboardGrab(ProtocolObjectId),
    ShortcutInhibitManager,
    ShortcutInhibitor(ProtocolObjectId),
    IdleInhibitManager,
    IdleInhibitor(ProtocolObjectId),
    PointerConstraints,
    LockedPointer(ProtocolObjectId),
    ConfinedPointer(ProtocolObjectId),
    ExplicitSynchronization,
    SurfaceSynchronization(WaylandSurfaceId),
    ExplicitBufferRelease(WaylandSurfaceId),
}

impl ResourceKind {
    fn object_kind(self) -> ProtocolObjectKind {
        match self {
            Self::ForeignToplevelList => ProtocolObjectKind::ForeignToplevelList,
            Self::ForeignToplevelHandle => ProtocolObjectKind::ForeignToplevelHandle,
            Self::ImageCopyCaptureManager => ProtocolObjectKind::ImageCopyCaptureManager,
            Self::ImageCopyCaptureSession(_) => ProtocolObjectKind::ImageCopyCaptureSession,
            Self::ImageCopyCaptureFrame(_) => ProtocolObjectKind::ImageCopyCaptureFrame,
            Self::OutputCaptureSourceManager => ProtocolObjectKind::OutputCaptureSourceManager,
            Self::OutputCaptureSource(_) => ProtocolObjectKind::ImageCaptureSource,
            Self::Compositor => ProtocolObjectKind::Compositor,
            Self::Surface(_) => ProtocolObjectKind::Surface,
            Self::Region(_) => ProtocolObjectKind::Region,
            Self::Shm => ProtocolObjectKind::Shm,
            Self::ShmPool(_) => ProtocolObjectKind::ShmPool,
            Self::Buffer(_) => ProtocolObjectKind::Buffer,
            Self::Subcompositor => ProtocolObjectKind::Subcompositor,
            Self::Subsurface(_) => ProtocolObjectKind::Subsurface,
            Self::XdgWmBase => ProtocolObjectKind::XdgWmBase,
            Self::XdgPositioner(_) => ProtocolObjectKind::XdgPositioner,
            Self::XdgSurface(_) => ProtocolObjectKind::XdgSurface,
            Self::XdgToplevel(_) => ProtocolObjectKind::XdgToplevel,
            Self::XdgPopup(_) => ProtocolObjectKind::XdgPopup,
            Self::XwaylandShell => ProtocolObjectKind::XwaylandShell,
            Self::XwaylandSurface(_) => ProtocolObjectKind::XwaylandSurface,
            Self::Callback(_) => ProtocolObjectKind::Callback,
            Self::Output(_) => ProtocolObjectKind::Output,
            Self::Seat(_) => ProtocolObjectKind::Seat,
            Self::Pointer(_) => ProtocolObjectKind::Pointer,
            Self::Keyboard(_) => ProtocolObjectKind::Keyboard,
            Self::Touch(_) => ProtocolObjectKind::Touch,
            Self::DataDeviceManager => ProtocolObjectKind::DataDeviceManager,
            Self::DataDevice(_) => ProtocolObjectKind::DataDevice,
            Self::DataSource(_) => ProtocolObjectKind::DataSource,
            Self::DataOffer(_) => ProtocolObjectKind::DataOffer,
            Self::LinuxDmaBuf => ProtocolObjectKind::LinuxDmaBuf,
            Self::LinuxDmaBufFeedback => ProtocolObjectKind::LinuxDmaBufFeedback,
            Self::LinuxBufferParams(_) => ProtocolObjectKind::LinuxBufferParams,
            Self::DecorationManager => ProtocolObjectKind::DecorationManager,
            Self::ToplevelDecoration(_) => ProtocolObjectKind::ToplevelDecoration,
            Self::CursorShapeManager => ProtocolObjectKind::CursorShapeManager,
            Self::CursorShapeDevice(_) => ProtocolObjectKind::CursorShapeDevice,
            Self::ToplevelIconManager => ProtocolObjectKind::ToplevelIconManager,
            Self::ToplevelIcon(_) => ProtocolObjectKind::ToplevelIcon,
            Self::FractionalScaleManager => ProtocolObjectKind::FractionalScaleManager,
            Self::FractionalScale => ProtocolObjectKind::FractionalScale,
            Self::Viewporter => ProtocolObjectKind::Viewporter,
            Self::Viewport(_) => ProtocolObjectKind::Viewport,
            Self::Presentation => ProtocolObjectKind::Presentation,
            Self::PresentationFeedback(_) => ProtocolObjectKind::PresentationFeedback,
            Self::Activation => ProtocolObjectKind::Activation,
            Self::ActivationToken(_) => ProtocolObjectKind::ActivationToken,
            Self::SessionLockManager => ProtocolObjectKind::SessionLockManager,
            Self::SessionLock(_) => ProtocolObjectKind::SessionLock,
            Self::SessionLockSurface(_) => ProtocolObjectKind::SessionLockSurface,
            Self::RelativePointerManager => ProtocolObjectKind::RelativePointerManager,
            Self::RelativePointer(_) => ProtocolObjectKind::RelativePointer,
            Self::XdgOutputManager => ProtocolObjectKind::XdgOutputManager,
            Self::XdgOutput(_, _) => ProtocolObjectKind::XdgOutput,
            Self::XwaylandKeyboardGrabManager => ProtocolObjectKind::XwaylandKeyboardGrabManager,
            Self::XwaylandKeyboardGrab(_) => ProtocolObjectKind::XwaylandKeyboardGrab,
            Self::ShortcutInhibitManager => ProtocolObjectKind::ShortcutInhibitManager,
            Self::ShortcutInhibitor(_) => ProtocolObjectKind::ShortcutInhibitor,
            Self::IdleInhibitManager => ProtocolObjectKind::IdleInhibitManager,
            Self::IdleInhibitor(_) => ProtocolObjectKind::IdleInhibitor,
            Self::PointerConstraints => ProtocolObjectKind::PointerConstraints,
            Self::LockedPointer(_) => ProtocolObjectKind::LockedPointer,
            Self::ConfinedPointer(_) => ProtocolObjectKind::ConfinedPointer,
            Self::ExplicitSynchronization => ProtocolObjectKind::ExplicitSynchronization,
            Self::SurfaceSynchronization(_) => ProtocolObjectKind::SurfaceSynchronization,
            Self::ExplicitBufferRelease(_) => ProtocolObjectKind::LinuxBufferRelease,
        }
    }
}

struct ResourceContext {
    state: *mut NativeState,
    object: ProtocolObjectId,
    client: ClientId,
    interface: String,
    kind: ResourceKind,
}

struct BindContext {
    state: *mut NativeState,
    interface: &'static str,
    kind: ResourceKind,
}

struct NativeShmPool {
    owner: ClientId,
    fd: OwnedFd,
    pool: ShmPool,
}

#[derive(Debug)]
struct NativeDmaBufPlane {
    fd: OwnedFd,
    offset: u32,
    stride: u32,
    modifier: u64,
}

#[derive(Debug, Default)]
struct NativeDmaBufParams {
    planes: BTreeMap<u32, NativeDmaBufPlane>,
    used: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufFormat {
    pub fourcc: u32,
    pub modifier: u64,
}

#[derive(Debug)]
pub struct DmaBufImage {
    pub descriptor: crate::integrations::wayland::compositor::DmaBufDescriptor,
    pub planes: Vec<OwnedFd>,
}

impl DmaBufImage {
    /// Snapshots the producer write fences carried by this DMA-BUF for a Vulkan read submission.
    ///
    /// Linux-DMA-BUF uses implicit synchronization unless a protocol extension supplies an
    /// explicit acquire fence. Vulkan itself is explicit-only, so the kernel reservation fences
    /// must be exported as a sync file before importing the image into a Vulkan command stream.
    pub fn export_implicit_read_sync_file(&self) -> Result<OwnedFd, NativeCompositorError> {
        let plane = self
            .planes
            .first()
            .ok_or_else(|| NativeCompositorError::new("DMA-BUF image has no plane file"))?;
        let mut export = crate::platform::linux::ffi::dma_buf_export_sync_file {
            flags: crate::platform::linux::ffi::DMA_BUF_SYNC_READ,
            fd: -1,
        };
        let result = unsafe {
            crate::platform::linux::ffi::ioctl(
                plane.as_raw_fd(),
                crate::platform::linux::ffi::DMA_BUF_IOCTL_EXPORT_SYNC_FILE,
                std::ptr::from_mut(&mut export),
            )
        };
        if result != 0 {
            return Err(NativeCompositorError::new(format!(
                "failed to export the DMA-BUF implicit read fence: {}",
                std::io::Error::last_os_error()
            )));
        }
        if export.fd < 0 {
            return Err(NativeCompositorError::new(
                "DMA-BUF implicit read-fence export returned no sync file",
            ));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(export.fd) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportSource {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportState {
    pub source: Option<ViewportSource>,
    pub destination: Option<crate::foundation::SizeI>,
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeViewport {
    current: ViewportState,
    pending_source: Option<Option<ViewportSource>>,
    pending_destination: Option<Option<crate::foundation::SizeI>>,
}

#[derive(Clone, Copy, Debug)]
struct NativeTouchPoint {
    client: ClientId,
    surface: WaylandSurfaceId,
    down_serial: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeDragGrab {
    Pointer,
    Touch(i32),
}

#[derive(Debug)]
struct NativeDragTarget {
    surface: WaylandSurfaceId,
    devices: Vec<ProtocolObjectId>,
    offers: Vec<ProtocolObjectId>,
}

#[derive(Debug)]
struct NativeDrag {
    seat: u32,
    source: Option<ProtocolObjectId>,
    origin: WaylandSurfaceId,
    icon: Option<WaylandSurfaceId>,
    grab: NativeDragGrab,
    target: Option<NativeDragTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerConstraintKind {
    Locked,
    Confined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerConstraintState {
    pub kind: PointerConstraintKind,
    pub surface: WaylandSurfaceId,
    pub region: Option<Region>,
}

#[derive(Clone, Debug)]
struct NativePointerConstraint {
    seat: u32,
    surface: WaylandSurfaceId,
    kind: PointerConstraintKind,
    region: Option<Region>,
    cursor_hint: Option<crate::foundation::PointF>,
    persistent: bool,
    active: bool,
    finished: bool,
}

#[derive(Debug, Default)]
struct NativeActivationToken {
    serial: Option<(u32, u32)>,
    application_id: Option<String>,
    surface: Option<WaylandSurfaceId>,
    committed: bool,
}

#[derive(Clone, Debug)]
struct NativeActivationGrant {
    authorized: bool,
    application_id: Option<String>,
    source_surface: Option<WaylandSurfaceId>,
}

#[derive(Debug)]
struct NativeSessionLock {
    client: ClientId,
    locked_event_sent: bool,
    finished_event_sent: bool,
}

#[derive(Debug)]
struct NativeSessionLockSurface {
    lock: ProtocolObjectId,
    output: u32,
    pending_configures: VecDeque<(u32, crate::foundation::SizeI)>,
    last_acked: Option<(u32, crate::foundation::SizeI)>,
}

#[derive(Debug, Default)]
struct NativeToplevelIcon {
    name: Option<String>,
    buffers: BTreeMap<(i32, i32), WaylandBufferId>,
    immutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToplevelIconImage {
    pub buffer: WaylandBufferId,
    pub scale: i32,
    pub image: ShmImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToplevelIconSnapshot {
    pub revision: u64,
    pub name: Option<String>,
    pub images: Vec<ToplevelIconImage>,
}

#[derive(Clone, Debug)]
enum PendingToplevelIcon {
    Reset,
    Icon(ToplevelIconSnapshot),
}

impl NativeViewport {
    fn commit(&mut self) {
        if let Some(source) = self.pending_source.take() {
            self.current.source = source;
        }
        if let Some(destination) = self.pending_destination.take() {
            self.current.destination = destination;
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeXdgPositioner {
    size: Option<crate::foundation::SizeI>,
    anchor_rect: Option<RectI>,
    anchor: u32,
    gravity: u32,
    constraint_adjustment: u32,
    offset: PointI,
    reactive: bool,
    parent_size: Option<crate::foundation::SizeI>,
    parent_configure: Option<u32>,
}

impl NativeXdgPositioner {
    fn finish(
        self,
    ) -> Result<crate::integrations::wayland::compositor::XdgPositioner, NativeCompositorError>
    {
        crate::integrations::wayland::compositor::XdgPositioner {
            size: self
                .size
                .ok_or_else(|| NativeCompositorError::new("positioner size was not set"))?,
            anchor_rect: self.anchor_rect.ok_or_else(|| {
                NativeCompositorError::new("positioner anchor rectangle was not set")
            })?,
            anchor: self.anchor,
            gravity: self.gravity,
            constraint_adjustment: self.constraint_adjustment,
            offset: self.offset,
            reactive: self.reactive,
            parent_size: self.parent_size,
            parent_configure: self.parent_configure,
        }
        .validate()
        .map_err(error)
    }
}

/// Shared owner-thread authorization across server generations. Replacing a live
/// client is rejected; the host must explicitly disconnect the old generation.
#[derive(Default)]
pub struct XwaylandAccess {
    coordinate_scale: Cell<i32>,
    display: Cell<usize>,
    client: RefCell<Option<Rc<OwnedClient>>>,
    generation: Cell<u64>,
    last_serial: Cell<u64>,
}
impl XwaylandAccess {
    /// One fixed X11 pixel density for this managed desktop; set before startup.
    #[cfg_attr(not(feature = "shell-xwayland"), allow(dead_code))]
    pub(crate) fn set_coordinate_scale(&self, scale: i32) {
        self.coordinate_scale.set(scale.clamp(1, 8));
    }
    pub(crate) fn coordinate_scale(&self) -> i32 {
        self.coordinate_scale.get().max(1)
    }

    /// Install the Xwayland registry policy before borrowing the display for
    /// compositor globals. The returned slot supports bounded server restarts.
    pub fn configure_display(display: &mut Display) -> Result<Rc<Self>, NativeCompositorError> {
        let pointer = NativeProtocol::desktop()
            .interface("xwayland_shell_v1")
            .ok_or_else(|| NativeCompositorError::new("missing Xwayland shell descriptor"))?
            as *const ffi::wl_interface;
        let grab_pointer = NativeProtocol::desktop()
            .interface("zwp_xwayland_keyboard_grab_manager_v1")
            .ok_or_else(|| NativeCompositorError::new("missing Xwayland grab descriptor"))?
            as *const ffi::wl_interface;
        let access = Rc::new(Self::default());
        access
            .display
            .set(display.native_handle().as_ptr() as usize);
        let policy = access.clone();
        display.add_global_filter(move |client, candidate| {
            (candidate != pointer && candidate != grab_pointer) || policy.allows(client)
        });
        Ok(access)
    }
    pub fn set_client(
        &self,
        client: Rc<OwnedClient>,
        generation: u64,
    ) -> Result<(), NativeCompositorError> {
        if generation <= self.generation.get()
            || !client.is_alive()
            || self
                .client
                .borrow()
                .as_ref()
                .is_some_and(|old| old.is_alive())
        {
            return Err(NativeCompositorError::new(
                "invalid Xwayland generation replacement",
            ));
        }
        *self.client.borrow_mut() = Some(client);
        self.generation.set(generation);
        self.last_serial.set(0);
        Ok(())
    }
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }
    fn allows(&self, identity: usize) -> bool {
        self.client
            .borrow()
            .as_ref()
            .is_some_and(|client| client.identity() == Some(identity))
    }
}

fn drag_source_actions(
    source_version: u32,
    offer_version: u32,
    actions: crate::integrations::wayland::compositor::DataAction,
) -> crate::integrations::wayland::compositor::DataAction {
    if source_version < 3 || offer_version < 3 {
        crate::integrations::wayland::compositor::DataAction::COPY
    } else {
        actions
    }
}

#[derive(Default)]
struct SuspendedFocus {
    keyboard: Option<Option<WaylandSurfaceId>>,
    pointer: Option<(Option<WaylandSurfaceId>, crate::foundation::PointF)>,
}

struct NativeState {
    foreign_toplevel: foreign_toplevel::NativeForeignToplevelState,
    capture: capture::NativeCaptureState,
    capture_access: Option<Rc<super::CaptureAccess>>,
    xwayland: Option<Rc<XwaylandAccess>>,
    display: std::ptr::NonNull<ffi::wl_display>,
    protocol: NativeProtocol,
    core: CompositorCore,
    output_revision: u64,
    clients: BTreeMap<usize, ClientId>,
    resources: BTreeMap<ProtocolObjectId, usize>,
    mapped_outputs: BTreeSet<WaylandSurfaceId>,
    entered_outputs: BTreeSet<(WaylandSurfaceId, ProtocolObjectId)>,
    regions: BTreeMap<ProtocolObjectId, Vec<RectI>>,
    shm_pools: BTreeMap<ProtocolObjectId, NativeShmPool>,
    buffer_files: BTreeMap<WaylandBufferId, OwnedFd>,
    dmabuf_files: BTreeMap<WaylandBufferId, Vec<OwnedFd>>,
    // Protocol destruction must not invalidate current, pending, or cached surface content.
    destroyed_buffers: BTreeMap<WaylandBufferId, ClientId>,
    callbacks: BTreeMap<WaylandSurfaceId, Vec<ProtocolObjectId>>,
    committed_callbacks: BTreeMap<(WaylandSurfaceId, u64), Vec<ProtocolObjectId>>,
    pending_presentation_feedbacks: BTreeMap<WaylandSurfaceId, Vec<ProtocolObjectId>>,
    committed_presentation_feedbacks: BTreeMap<(WaylandSurfaceId, u64), Vec<ProtocolObjectId>>,
    xdg_resources: BTreeMap<WaylandSurfaceId, ProtocolObjectId>,
    toplevels: BTreeMap<WaylandSurfaceId, XdgToplevelState>,
    decoration_policy: crate::DecorationPolicy,
    committed_decorations:
        BTreeMap<WaylandSurfaceId, crate::integrations::wayland::compositor::DecorationMode>,
    toplevel_icons: BTreeMap<ProtocolObjectId, NativeToplevelIcon>,
    pending_toplevel_icons: BTreeMap<WaylandSurfaceId, PendingToplevelIcon>,
    committed_toplevel_icons: BTreeMap<WaylandSurfaceId, ToplevelIconSnapshot>,
    positioners: BTreeMap<ProtocolObjectId, NativeXdgPositioner>,
    popups: BTreeMap<WaylandSurfaceId, crate::integrations::wayland::compositor::XdgPopupState>,
    viewports: BTreeMap<WaylandSurfaceId, NativeViewport>,
    dmabuf_formats: Vec<DmaBufFormat>,
    dmabuf_feedback: Option<DmaBufFeedback>,
    dmabuf_params: BTreeMap<ProtocolObjectId, NativeDmaBufParams>,
    keyboard_keymaps: BTreeMap<u32, (OwnedFd, u32)>,
    touch_points: BTreeMap<(u32, i32), NativeTouchPoint>,
    active_drag: Option<NativeDrag>,
    finished_drag_sources: BTreeSet<ProtocolObjectId>,
    xwayland_keyboard_grabs: BTreeMap<ProtocolObjectId, (u32, WaylandSurfaceId, bool)>,
    shortcut_inhibitors: BTreeMap<ProtocolObjectId, (u32, WaylandSurfaceId, bool)>,
    revoked_shortcuts: BTreeSet<(u32, WaylandSurfaceId)>,
    idle_inhibitors: BTreeMap<ProtocolObjectId, WaylandSurfaceId>,
    pointer_constraints: BTreeMap<ProtocolObjectId, NativePointerConstraint>,
    pointer_capture_releases: BTreeMap<u32, WaylandSurfaceId>,
    suspended_focus: BTreeMap<u32, SuspendedFocus>,
    pointer_press_serials:
        BTreeMap<(u32, u32), (u32, crate::integrations::wayland::compositor::PointerFocus)>,
    activation_tokens: BTreeMap<ProtocolObjectId, NativeActivationToken>,
    activation_grants: BTreeMap<String, NativeActivationGrant>,
    activation_order: VecDeque<String>,
    session_locks: BTreeMap<ProtocolObjectId, NativeSessionLock>,
    session_lock_surfaces: BTreeMap<WaylandSurfaceId, NativeSessionLockSurface>,
    active_session_lock: Option<ProtocolObjectId>,
    secure_session_locked: bool,
    synchronized_surfaces: BTreeSet<WaylandSurfaceId>,
    pending_acquire_fences: BTreeMap<WaylandSurfaceId, OwnedFd>,
    pending_releases: BTreeMap<WaylandSurfaceId, ProtocolObjectId>,
    committed_acquire_fences: BTreeMap<(WaylandSurfaceId, u64), OwnedFd>,
    committed_releases: BTreeMap<(WaylandSurfaceId, u64), ProtocolObjectId>,
    initial_configures: BTreeSet<WaylandSurfaceId>,
    next_client: u32,
    next_object: u32,
    next_surface: u32,
    next_buffer: u32,
    presentation_sequence: u64,
    toplevel_icon_revision: u64,
}

pub struct NativeCompositor<'display> {
    state: Box<NativeState>,
    // libwayland retains pointers to these values, so each context needs an allocation whose
    // address is stable even when this vector grows.
    #[allow(clippy::vec_box)]
    bind_contexts: Vec<Box<BindContext>>,
    globals: Vec<Global<'display>>,
}

impl Drop for NativeCompositor<'_> {
    fn drop(&mut self) {
        // Every resource context retains a pointer into `state`. Disconnect clients while that
        // state is still alive so libwayland's resource-destroy callbacks cannot dereference it
        // after this owner has been dropped. `Display::drop` may repeat this on an empty list.
        unsafe { ffi::wl_display_destroy_clients(self.state.display.as_ptr()) };
        // Libwayland also retains each bind callback and its data pointer in the corresponding
        // global. Destroy the globals before releasing those callback contexts.
        self.globals.clear();
        self.bind_contexts.clear();
    }
}

#[derive(Default)]
struct DispatchOutcome {
    destroy_self: bool,
    destroy_others: Vec<usize>,
}

fn c_string(request: &IncomingRequest<'_>, index: usize) -> Result<String, NativeCompositorError> {
    Ok(request
        .string(index)
        .map_err(error)?
        .ok_or_else(|| NativeCompositorError::new("string must not be null"))?
        .to_string_lossy()
        .into_owned())
}

fn protocol_string(value: &str) -> std::ffi::CString {
    std::ffi::CString::new(value)
        .unwrap_or_else(|_| std::ffi::CString::new(value.replace('\0', "")).expect("sanitized"))
}

fn activation_token_handle() -> Result<String, NativeCompositorError> {
    let mut random = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| {
            NativeCompositorError::new(format!("activation-token entropy failed: {error}"))
        })?;
    let mut handle = String::with_capacity(7 + random.len() * 2);
    handle.push_str("telorgon-");
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut handle, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Ok(handle)
}

fn fd_size(fd: &OwnedFd) -> std::io::Result<u64> {
    let file = std::fs::File::from(fd.try_clone()?);
    Ok(file.metadata()?.len())
}

fn unsupported_request(request: &IncomingRequest<'_>) -> NativeCompositorError {
    NativeCompositorError::new(format!(
        "request {} is not implemented",
        request.message().name
    ))
}

fn error(error: impl fmt::Display) -> NativeCompositorError {
    NativeCompositorError::new(error.to_string())
}

#[derive(Debug)]
pub struct NativeCompositorError {
    context: String,
}

impl NativeCompositorError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
        }
    }
}

impl fmt::Display for NativeCompositorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl std::error::Error for NativeCompositorError {}

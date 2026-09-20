//! Side-effect-free translation of lifecycle/metrics-related Winit window observations.

use std::error::Error;
use std::fmt;

use crate::platform::contracts::{
    CloseRequest, CloseRequestReason, ForcedDestruction, ForcedDestructionPhase, PhysicalExtent,
    ScaleFactor, ViewId, ViewSnapshot,
};
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::window::WindowId;

use crate::platform::winit::ViewRegistry;

/// Minimal copied fact selected from one supported Winit window event.
///
/// Scale-factor selection deliberately omits Winit's `InnerSizeWriter`; this boundary never invokes
/// it or retains it beyond the native callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WinitWindowFact {
    /// The client area's physical-pixel dimensions changed. Either dimension may be zero.
    Resized {
        /// Winit's physical client-area size.
        physical_size: PhysicalSize<u32>,
    },
    /// Winit reported a new logical-to-physical scale factor.
    ScaleFactorChanged {
        /// Original Winit `f64` observation before neutral validation/narrowing.
        scale_factor: f64,
    },
    /// The native window gained or lost keyboard focus.
    Focused {
        /// Whether the native window is focused.
        focused: bool,
    },
    /// Winit reported complete occlusion or the end of complete occlusion.
    Occluded {
        /// Whether the window is completely occluded.
        occluded: bool,
    },
    /// The native window manager requested cancellable close handling.
    CloseRequested,
    /// Winit reports that native destruction has completed.
    Destroyed,
}

impl WinitWindowFact {
    /// Copies the supported portion of a borrowed Winit event without invoking event-owned handles.
    ///
    /// Events owned by input, IME, data transfer, theme, redraw, or later adapter slices return
    /// `None` and remain untouched for their dedicated translator.
    pub fn from_event(event: &WindowEvent) -> Option<Self> {
        match event {
            WindowEvent::Resized(physical_size) => Some(Self::Resized {
                physical_size: *physical_size,
            }),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                Some(Self::ScaleFactorChanged {
                    scale_factor: *scale_factor,
                })
            }
            WindowEvent::Focused(focused) => Some(Self::Focused { focused: *focused }),
            WindowEvent::Occluded(occluded) => Some(Self::Occluded {
                occluded: *occluded,
            }),
            WindowEvent::CloseRequested => Some(Self::CloseRequested),
            WindowEvent::Destroyed => Some(Self::Destroyed),
            _ => None,
        }
    }
}

/// Neutral or adapter-local meaning copied from one supported Winit fact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WinitWindowObservationKind {
    /// Physical client-area dimensions, preserving explicit zero extent.
    Resized {
        /// Neutral unsigned physical-pixel extent.
        physical_extent: PhysicalExtent,
    },
    /// Validated logical-to-physical scale factor.
    ScaleFactorChanged {
        /// Neutral validated scale factor.
        scale_factor: ScaleFactor,
    },
    /// Native focus changed; input/focus owners decide the resulting reset or restoration work.
    FocusChanged {
        /// Whether native focus is present.
        focused: bool,
    },
    /// Complete occlusion changed; lifecycle owners combine it with their other visibility facts.
    OcclusionChanged {
        /// Whether the window is completely occluded.
        occluded: bool,
    },
    /// Revision-bound cancellable user close request.
    CloseRequested(CloseRequest),
    /// Revision-bound, unanswerable completed native destruction notification.
    Destroyed(ForcedDestruction),
}

/// One immutable, view-scoped observation selected from a current Winit window callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WinitWindowObservation {
    source_window: WindowId,
    view: ViewId,
    kind: WinitWindowObservationKind,
}

impl WinitWindowObservation {
    /// Returns the exact Winit identity that produced the copied fact.
    pub const fn source_window(self) -> WindowId {
        self.source_window
    }

    /// Returns the exact logical view generation resolved at translation time.
    pub const fn view(self) -> ViewId {
        self.view
    }

    /// Returns the copied and validated observation kind.
    pub const fn kind(self) -> WinitWindowObservationKind {
        self.kind
    }
}

/// Typed rejection from contextual Winit window translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowTranslationError {
    /// The callback's Winit identity is stale, retired, or unknown.
    WindowUnavailable {
        /// Unresolvable native identity.
        window: WindowId,
    },
    /// The caller supplied a snapshot for a different logical view than the current registry map.
    SnapshotViewMismatch {
        /// Current native identity.
        window: WindowId,
        /// Logical view currently registered to the native identity.
        registered_view: ViewId,
        /// Logical view cited by the supplied snapshot.
        snapshot_view: ViewId,
    },
    /// Winit's `f64` scale observation cannot enter the validated neutral `f32` representation.
    InvalidScaleFactor {
        /// Exact view generation for which scale was reported.
        view: ViewId,
        /// Original Winit observation.
        observed: f64,
    },
}

impl fmt::Display for WindowTranslationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowUnavailable { window } => write!(
                formatter,
                "Winit window {window:?} is stale, retired, or unknown during translation"
            ),
            Self::SnapshotViewMismatch {
                window,
                registered_view,
                snapshot_view,
            } => write!(
                formatter,
                "Winit window {window:?} belongs to {registered_view}, not snapshot view {snapshot_view}"
            ),
            Self::InvalidScaleFactor { view, observed } => {
                write!(
                    formatter,
                    "Winit view {view} reported invalid scale factor {observed}"
                )
            }
        }
    }
}

impl Error for WindowTranslationError {}

/// Selects and translates a supported borrowed Winit event.
///
/// Unsupported events return `Ok(None)` without being consumed or modified. Supported events are
/// validated against the current bidirectional registry before a copied observation is returned.
pub fn translate_window_event(
    registry: &ViewRegistry,
    source_window: WindowId,
    snapshot: &ViewSnapshot,
    event: &WindowEvent,
) -> Result<Option<WinitWindowObservation>, WindowTranslationError> {
    let Some(fact) = WinitWindowFact::from_event(event) else {
        return Ok(None);
    };
    translate_window_fact(registry, source_window, snapshot, fact).map(Some)
}

/// Translates one already-copied supported Winit fact.
///
/// This function performs no canonical state mutation, dispatch, close decision, native call, or
/// event-loop policy. A close request and forced destruction cite the supplied exact view snapshot
/// revision but remain distinct values.
pub fn translate_window_fact(
    registry: &ViewRegistry,
    source_window: WindowId,
    snapshot: &ViewSnapshot,
    fact: WinitWindowFact,
) -> Result<WinitWindowObservation, WindowTranslationError> {
    let registered_view = registry.view_for_window(source_window).ok_or(
        WindowTranslationError::WindowUnavailable {
            window: source_window,
        },
    )?;
    if registered_view != snapshot.view() {
        return Err(WindowTranslationError::SnapshotViewMismatch {
            window: source_window,
            registered_view,
            snapshot_view: snapshot.view(),
        });
    }

    let kind = match fact {
        WinitWindowFact::Resized { physical_size } => WinitWindowObservationKind::Resized {
            physical_extent: PhysicalExtent::new(physical_size.width, physical_size.height),
        },
        WinitWindowFact::ScaleFactorChanged { scale_factor } => {
            let scale_factor = ScaleFactor::new(scale_factor as f32).map_err(|_| {
                WindowTranslationError::InvalidScaleFactor {
                    view: registered_view,
                    observed: scale_factor,
                }
            })?;
            WinitWindowObservationKind::ScaleFactorChanged { scale_factor }
        }
        WinitWindowFact::Focused { focused } => {
            WinitWindowObservationKind::FocusChanged { focused }
        }
        WinitWindowFact::Occluded { occluded } => {
            WinitWindowObservationKind::OcclusionChanged { occluded }
        }
        WinitWindowFact::CloseRequested => WinitWindowObservationKind::CloseRequested(
            CloseRequest::from_snapshot(snapshot, CloseRequestReason::User),
        ),
        WinitWindowFact::Destroyed => WinitWindowObservationKind::Destroyed(
            ForcedDestruction::from_snapshot(snapshot, ForcedDestructionPhase::Destroyed),
        ),
    };

    Ok(WinitWindowObservation {
        source_window,
        view: registered_view,
        kind,
    })
}

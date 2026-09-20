//! Protocol-neutral shell-domain mounting primitives.
//!
//! Policy, native protocol objects, renderer resources, event loops, and background work remain
//! outside this crate.

pub mod client_surface;
pub mod diagnostics;
pub mod drag_region;
pub mod exclusive_region;
pub mod ext;
pub mod layer;
pub mod output_edge;
pub mod output_view;
pub mod placeholder;
pub mod prelude;
pub mod reserved_area;
pub mod resize_region;
pub mod root;
pub mod snapshot;
pub mod surface_input_region;
pub mod surface_tree;

pub use client_surface::*;
pub use diagnostics::*;
pub use drag_region::*;
pub use exclusive_region::*;
pub use ext::*;
pub use layer::*;
pub use output_edge::*;
pub use output_view::*;
pub use placeholder::*;
pub use reserved_area::*;
pub use resize_region::*;
pub use root::*;
pub use snapshot::*;
pub use surface_input_region::*;
pub use surface_tree::*;

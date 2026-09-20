//! Telorgon SDK: curated application API and explicit subsystem owners.

extern crate self as telorgon;

mod api;
pub mod assets;
pub mod authoring;
pub mod components;
pub mod foundation;
pub mod graphics;
pub mod host;
pub mod input;
pub mod integrations;
pub mod platform;
pub mod runtime;
pub mod services;
pub mod shell;
pub mod theme;
pub mod ui;

pub use api::*;

#[cfg(test)]
pub(crate) mod test_alloc;

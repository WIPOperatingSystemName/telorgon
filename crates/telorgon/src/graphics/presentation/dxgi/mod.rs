//! Windows D3D11/DXGI native presentation, independent of the rendering API.

#[cfg(target_os = "windows")]
mod windows_presenter;

#[cfg(target_os = "windows")]
pub use windows_presenter::{DxgiPresentFailure, DxgiPresenter};

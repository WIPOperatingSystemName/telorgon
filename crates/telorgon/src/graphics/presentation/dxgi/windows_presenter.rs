use crate::foundation::SizeI;
use crate::graphics::presentation::{
    PresentationError, PresentationErrorKind, PresentationRecovery, PresentationResult,
    PresentationState, is_zero_extent,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{
    CloseHandle, GENERIC_ALL, HANDLE, HMODULE, HWND, LUID, WAIT_OBJECT_0,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FENCE_FLAG_SHARED, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext4, ID3D11Fence,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT,
    DXGI_PRESENT_PARAMETERS, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter1,
    IDXGIFactory2, IDXGIFactory4, IDXGISwapChain1, IDXGISwapChain3,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::Interface;

const BUFFER_COUNT: u32 = 2;

#[derive(Debug)]
pub struct DxgiPresentFailure {
    error: PresentationError,
    completion_unproven: bool,
}

impl DxgiPresentFailure {
    fn recoverable(error: PresentationError) -> Self {
        Self {
            error,
            completion_unproven: false,
        }
    }

    fn completion_unproven(error: PresentationError) -> Self {
        Self {
            error,
            completion_unproven: true,
        }
    }

    pub const fn has_completion_proof(&self) -> bool {
        !self.completion_unproven
    }

    pub fn into_error(self) -> PresentationError {
        self.error
    }
}

impl std::fmt::Display for DxgiPresentFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for DxgiPresentFailure {}

/// Owns the compositor-facing D3D11 device, HWND swapchain, and native present operations.
pub struct DxgiPresenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext4,
    factory: IDXGIFactory4,
    swapchain: Option<IDXGISwapChain3>,
    recovery: PresentationRecovery,
    hwnd: isize,
}

impl DxgiPresenter {
    pub fn new(
        window: &impl HasWindowHandle,
        adapter_luid: [u8; 8],
        extent: SizeI,
    ) -> PresentationResult<Self> {
        let hwnd = hwnd(window)?.0 as isize;
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
            .map_err(|error| win_error("failed to create DXGI factory", error))?;
        let adapter_luid = LUID {
            LowPart: u32::from_ne_bytes(
                adapter_luid[0..4]
                    .try_into()
                    .expect("four-byte LUID low part"),
            ),
            HighPart: i32::from_ne_bytes(
                adapter_luid[4..8]
                    .try_into()
                    .expect("four-byte LUID high part"),
            ),
        };
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(adapter_luid) }
            .map_err(|error| win_error("failed to find rendering adapter in DXGI", error))?;
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|error| win_error("failed to create same-adapter D3D11 device", error))?;
        let device = device.expect("D3D11 returned a device on success");
        let context = context
            .expect("D3D11 returned a context on success")
            .cast::<ID3D11DeviceContext4>()
            .map_err(|error| win_error("D3D11.4 context is unavailable", error))?;
        let mut presenter = Self {
            device,
            context,
            factory,
            swapchain: None,
            recovery: PresentationRecovery::new(extent),
            hwnd,
        };
        if !is_zero_extent(extent) {
            presenter.reconfigure()?;
        }
        Ok(presenter)
    }

    pub const fn recovery(&self) -> PresentationRecovery {
        self.recovery
    }

    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub fn resize(&mut self, extent: SizeI) -> PresentationResult<bool> {
        if self.recovery.state() == PresentationState::Shutdown {
            return Err(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "cannot resize a shut down DXGI presenter",
            ));
        }
        let changed = self.recovery.resize(extent);
        if is_zero_extent(extent) {
            self.swapchain = None;
            self.recovery.state = PresentationState::Suspended;
        } else if changed {
            self.reconfigure()?;
        }
        Ok(changed)
    }

    pub fn suspend(&mut self) {
        if self.recovery.state() == PresentationState::Shutdown {
            return;
        }
        self.swapchain = None;
        self.recovery.state = PresentationState::Suspended;
    }

    pub fn create_shared_fence(&self) -> PresentationResult<(ID3D11Fence, HANDLE)> {
        let device5 = self
            .device
            .cast::<ID3D11Device5>()
            .map_err(|error| win_error("D3D11.4 device is unavailable", error))?;
        let mut fence: Option<ID3D11Fence> = None;
        unsafe { device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence) }
            .map_err(|error| win_error("failed to create shared D3D fence", error))?;
        let fence = fence.expect("D3D11 returned a fence on success");
        let handle = unsafe { fence.CreateSharedHandle(None, GENERIC_ALL.0, None) }
            .map_err(|error| win_error("failed to share D3D fence", error))?;
        Ok((fence, handle))
    }

    pub fn wait_for_fence(&self, fence: &ID3D11Fence, value: u64) -> PresentationResult<()> {
        if value == 0 || unsafe { fence.GetCompletedValue() } >= value {
            return Ok(());
        }
        let event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|error| win_error("failed to create DXGI fence event", error))?;
        let wait_result = unsafe {
            if let Err(error) = fence.SetEventOnCompletion(value, event) {
                let _ = CloseHandle(event);
                return Err(win_error("failed to arm DXGI fence event", error));
            }
            WaitForSingleObject(event, INFINITE)
        };
        let _ = unsafe { CloseHandle(event) };
        if wait_result != WAIT_OBJECT_0 {
            return Err(PresentationError::new(
                PresentationErrorKind::Native,
                "waiting for DXGI fence completion failed",
            ));
        }
        Ok(())
    }

    /// Copies one D3D11-compatible source into the current HWND back buffer and presents it.
    ///
    /// Cross-API mutex ownership remains the caller's responsibility. The presenter owns the D3D
    /// wait, copy, native present, and completion signal as one ordered operation.
    pub fn present_shared_texture(
        &mut self,
        source: &ID3D11Texture2D,
        fence: &ID3D11Fence,
        wait_value: u64,
        signal_value: u64,
    ) -> Result<(), DxgiPresentFailure> {
        let swapchain = self.swapchain.as_ref().ok_or_else(|| {
            DxgiPresentFailure::recoverable(PresentationError::new(
                PresentationErrorKind::InvalidState,
                "DXGI swapchain is unavailable",
            ))
        })?;
        let back_buffer_index = unsafe { swapchain.GetCurrentBackBufferIndex() };
        let back_buffer: ID3D11Texture2D = unsafe { swapchain.GetBuffer(back_buffer_index) }
            .map_err(|error| {
                DxgiPresentFailure::recoverable(win_error(
                    "failed to acquire DXGI back buffer",
                    error,
                ))
            })?;
        unsafe {
            self.context.Wait(fence, wait_value).map_err(|error| {
                DxgiPresentFailure::recoverable(win_error("failed to queue D3D fence wait", error))
            })?;
            self.context.CopyResource(&back_buffer, source);
        }
        let parameters = DXGI_PRESENT_PARAMETERS::default();
        let presented = unsafe { swapchain.Present1(1, DXGI_PRESENT(0), &parameters) };
        unsafe { self.context.Signal(fence, signal_value) }.map_err(|error| {
            DxgiPresentFailure::completion_unproven(win_error(
                "failed to signal D3D copy completion",
                error,
            ))
        })?;
        presented.ok().map_err(|error| {
            DxgiPresentFailure::recoverable(win_error("DXGI presentation failed", error))
        })
    }

    pub fn shutdown(&mut self) {
        self.swapchain = None;
        self.recovery.state = PresentationState::Shutdown;
    }

    fn reconfigure(&mut self) -> PresentationResult<()> {
        let extent = self.recovery.requested_extent;
        if is_zero_extent(extent) {
            self.suspend();
            return Ok(());
        }
        let width = extent.width as u32;
        let height = extent.height as u32;
        if let Some(swapchain) = self.swapchain.as_ref() {
            unsafe {
                swapchain.ResizeBuffers(
                    BUFFER_COUNT,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
            }
            .map_err(|error| win_error("failed to resize DXGI swapchain", error))?;
        } else {
            self.swapchain = Some(create_swapchain(
                &self.factory,
                &self.device,
                HWND(self.hwnd as *mut _),
                width,
                height,
            )?);
        }
        self.recovery.mark_reconfigured()?;
        Ok(())
    }
}

fn create_swapchain(
    factory: &IDXGIFactory2,
    device: &ID3D11Device,
    hwnd: HWND,
    width: u32,
    height: u32,
) -> PresentationResult<IDXGISwapChain3> {
    let desc = swapchain_desc(width, height);
    let swapchain: IDXGISwapChain1 =
        unsafe { factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None) }
            .map_err(|error| win_error("failed to create scaling-none DXGI swapchain", error))?;
    unsafe { factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) }
        .map_err(|error| win_error("failed to configure DXGI window association", error))?;
    swapchain
        .cast()
        .map_err(|error| win_error("DXGI swapchain 3 is unavailable", error))
}

fn swapchain_desc(width: u32, height: u32) -> DXGI_SWAP_CHAIN_DESC1 {
    DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: BUFFER_COUNT,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    }
}

fn hwnd(window: &impl HasWindowHandle) -> PresentationResult<HWND> {
    let handle = window.window_handle().map_err(|error| {
        PresentationError::new(
            PresentationErrorKind::Unsupported,
            format!("native window handle is unavailable: {error}"),
        )
    })?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut _)),
        _ => Err(PresentationError::new(
            PresentationErrorKind::Unsupported,
            "DXGI presentation requires a Win32 HWND",
        )),
    }
}

fn win_error(context: &str, error: windows::core::Error) -> PresentationError {
    PresentationError::with_backend_code(
        PresentationErrorKind::Native,
        format!("{context}: {error}"),
        i64::from(error.code().0),
    )
}

#[cfg(test)]
mod tests {
    use super::swapchain_desc;
    use windows::Win32::Graphics::Dxgi::{DXGI_SCALING_NONE, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL};

    #[test]
    fn hwnd_swapchain_disables_scaling() {
        let desc = swapchain_desc(1_280, 720);
        assert_eq!(desc.Width, 1_280);
        assert_eq!(desc.Height, 720);
        assert_eq!(desc.BufferCount, 2);
        assert_eq!(desc.Scaling, DXGI_SCALING_NONE);
        assert_eq!(desc.SwapEffect, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL);
    }
}

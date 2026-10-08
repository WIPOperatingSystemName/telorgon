mod pixels;

pub use pixels::{ChannelMasks, FramebufferLayout, PixelFormat, convert_rgba8};

use alloc::vec::Vec;
use core::ptr::{self, NonNull};
use r_efi::protocols::graphics_output as gop;

use crate::platform::uefi::{UefiContext, UefiError, UefiResult};

pub struct UefiPresenter<'a> {
    context: &'a UefiContext,
    protocol: NonNull<gop::Protocol>,
    layout: FramebufferLayout,
    framebuffer: *mut u8,
    blt_row: Vec<gop::BltPixel>,
    presentation_epoch: u64,
}

const MAX_GOP_MODES: u32 = 4096;

impl<'a> UefiPresenter<'a> {
    pub fn new(context: &'a UefiContext) -> UefiResult<Self> {
        let protocol = context.locate_protocol::<gop::Protocol>(gop::PROTOCOL_GUID)?;
        let mut result = Self {
            context,
            protocol,
            layout: FramebufferLayout::new(1, 1, 1, 0, PixelFormat::BltOnly)?,
            framebuffer: ptr::null_mut(),
            blt_row: Vec::new(),
            presentation_epoch: context.presentation_epoch(),
        };
        result.refresh()?;
        Ok(result)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.layout.width(), self.layout.height())
    }

    /// Selects an exact supported resolution. Returns true when it is active,
    /// including when already selected, or false when no advertised mode matches.
    pub fn set_resolution(&mut self, width: u32, height: u32) -> UefiResult<bool> {
        self.context.ensure_active()?;
        if width == 0 || height == 0 {
            return Err(UefiError::InvalidFramebuffer);
        }
        self.refresh()?;
        let mode = NonNull::new(unsafe { self.protocol.as_ref().mode })
            .ok_or(UefiError::InvalidFramebuffer)?;
        let maximum = unsafe { mode.as_ref().max_mode };
        if maximum == 0 {
            return Err(UefiError::InvalidFramebuffer);
        }
        if maximum > MAX_GOP_MODES {
            return Err(UefiError::ResourceLimit);
        }
        if self.size() == (width, height) {
            return Ok(true);
        }
        for index in 0..maximum {
            let information = self.query_mode(index)?;
            if information.horizontal_resolution != width
                || information.vertical_resolution != height
            {
                continue;
            }
            let status =
                unsafe { (self.protocol.as_ref().set_mode)(self.protocol.as_ptr(), index) };
            // Even a failed mode change must invalidate cached device pointers.
            // Other presenters sharing this context refresh before their next write.
            self.context.invalidate_presentation();
            crate::platform::uefi::check("set graphics mode", status)?;
            self.refresh()?;
            if self.size() != (width, height) {
                return Err(UefiError::InvalidFramebuffer);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn query_mode(&self, index: u32) -> UefiResult<gop::ModeInformation> {
        let services = self.context.services()?;
        let mut size = 0;
        let mut information = ptr::null_mut();
        crate::platform::uefi::check("query graphics mode", unsafe {
            (self.protocol.as_ref().query_mode)(
                self.protocol.as_ptr(),
                index,
                &mut size,
                &mut information,
            )
        })?;
        let information = NonNull::new(information).ok_or(UefiError::InvalidFramebuffer)?;
        // QueryMode transfers a pool allocation to its caller. Copy the known
        // prefix and release it before changing the hardware mode.
        let snapshot = if size >= core::mem::size_of::<gop::ModeInformation>() {
            Some(unsafe { ptr::read(information.as_ptr()) })
        } else {
            None
        };
        crate::platform::uefi::check("release graphics mode information", unsafe {
            (services.free_pool)(information.as_ptr().cast())
        })?;
        snapshot.ok_or(UefiError::InvalidFramebuffer)
    }

    pub fn framebuffer_layout(&self) -> Option<FramebufferLayout> {
        if !self.context.boot_services_active()
            || self.presentation_epoch != self.context.presentation_epoch()
            || self.layout.pixel_format() == PixelFormat::BltOnly
        {
            None
        } else {
            Some(self.layout)
        }
    }

    pub fn framebuffer_address(&self) -> Option<u64> {
        self.framebuffer_layout()
            .map(|_| self.framebuffer as usize as u64)
    }

    /// Re-query after a returning EFI application; it may have changed GOP.
    pub fn refresh(&mut self) -> UefiResult<()> {
        self.context.ensure_active()?;
        // A failed refresh must never allow writes using a partly updated mode.
        self.presentation_epoch = self.context.presentation_epoch().wrapping_sub(1);
        self.protocol = self.context.locate_protocol(gop::PROTOCOL_GUID)?;
        let mode = NonNull::new(unsafe { self.protocol.as_ref().mode })
            .ok_or(UefiError::InvalidFramebuffer)?;
        let mode = unsafe { mode.as_ref() };
        if mode.size_of_info < core::mem::size_of::<gop::ModeInformation>() {
            return Err(UefiError::InvalidFramebuffer);
        }
        let info = NonNull::new(mode.info).ok_or(UefiError::InvalidFramebuffer)?;
        let info = unsafe { info.as_ref() };
        let format = match info.pixel_format {
            gop::PIXEL_RED_GREEN_BLUE_RESERVED_8_BIT_PER_COLOR => PixelFormat::RgbReserved,
            gop::PIXEL_BLUE_GREEN_RED_RESERVED_8_BIT_PER_COLOR => PixelFormat::BgrReserved,
            gop::PIXEL_BIT_MASK => PixelFormat::BitMask(ChannelMasks {
                red: info.pixel_information.red_mask,
                green: info.pixel_information.green_mask,
                blue: info.pixel_information.blue_mask,
                reserved: info.pixel_information.reserved_mask,
            }),
            gop::PIXEL_BLT_ONLY => PixelFormat::BltOnly,
            _ => return Err(UefiError::InvalidPixelFormat),
        };
        self.layout = FramebufferLayout::new(
            info.horizontal_resolution,
            info.vertical_resolution,
            info.pixels_per_scan_line,
            mode.frame_buffer_size,
            format,
        )?;
        if format != PixelFormat::BltOnly {
            let address = usize::try_from(mode.frame_buffer_base)
                .map_err(|_| UefiError::InvalidFramebuffer)?;
            if address == 0 || address.checked_add(self.layout.required_bytes()).is_none() {
                return Err(UefiError::InvalidFramebuffer);
            }
            self.framebuffer = address as *mut u8;
        } else {
            self.framebuffer = ptr::null_mut();
            let width = self.layout.width() as usize;
            if self.blt_row.capacity() < width {
                self.blt_row
                    .try_reserve_exact(width.saturating_sub(self.blt_row.len()))
                    .map_err(|_| UefiError::ResourceLimit)?;
            }
            self.blt_row.resize(
                width,
                gop::BltPixel {
                    blue: 0,
                    green: 0,
                    red: 0,
                    reserved: 0,
                },
            );
        }
        self.presentation_epoch = self.context.presentation_epoch();
        Ok(())
    }

    pub fn present_rgba8(&mut self, width: u32, height: u32, rgba: &[u8]) -> UefiResult<()> {
        self.context.ensure_active()?;
        if self.presentation_epoch != self.context.presentation_epoch() {
            self.refresh()?;
        }
        self.layout.validate_source(width, height, rgba.len())?;
        let width = width as usize;
        if self.layout.pixel_format() == PixelFormat::BltOnly {
            for row in 0..height as usize {
                for (column, output) in self.blt_row.iter_mut().enumerate() {
                    let source = &rgba[(row * width + column) * 4..][..4];
                    *output = gop::BltPixel {
                        blue: source[2],
                        green: source[1],
                        red: source[0],
                        reserved: 0,
                    };
                }
                let status = unsafe {
                    (self.protocol.as_ref().blt)(
                        self.protocol.as_ptr(),
                        self.blt_row.as_mut_ptr(),
                        gop::BLT_BUFFER_TO_VIDEO,
                        0,
                        0,
                        0,
                        row,
                        width,
                        1,
                        width * 4,
                    )
                };
                crate::platform::uefi::check("GOP blit", status)?;
            }
        } else {
            let pixel_bytes = self.layout.pixel_bytes();
            for row in 0..height as usize {
                for column in 0..width {
                    let source = &rgba[(row * width + column) * 4..][..4];
                    let encoded = self
                        .layout
                        .encode_pixel([source[0], source[1], source[2], source[3]]);
                    let offset = row * self.layout.stride_bytes() + column * pixel_bytes;
                    let target = unsafe { self.framebuffer.add(offset) };
                    // Raw volatile writes never create aliased Rust references to
                    // device memory. Reserved channels receive zero, not alpha.
                    unsafe {
                        if pixel_bytes == 4 && (target as usize).is_multiple_of(4) {
                            ptr::write_volatile(target.cast::<u32>(), encoded);
                        } else if pixel_bytes == 2 && (target as usize).is_multiple_of(2) {
                            ptr::write_volatile(target.cast::<u16>(), encoded as u16);
                        } else {
                            for (index, byte) in
                                encoded.to_le_bytes().iter().take(pixel_bytes).enumerate()
                            {
                                ptr::write_volatile(target.add(index), *byte);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

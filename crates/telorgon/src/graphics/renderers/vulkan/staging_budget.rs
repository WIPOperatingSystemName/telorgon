//! Upload capacity for managed native surfaces, shared by applications and the shell.
use crate::{
    SizeI,
    graphics::render::{RenderError, RenderErrorKind, RenderResult},
};

pub(crate) const MIN_BYTES_PER_SLOT: u64 = 16 * 1024 * 1024;
pub(crate) const HEADROOM_BYTES_PER_SLOT: u64 = 16 * 1024 * 1024;

pub(crate) fn surface_budget(extent: SizeI, frame_slots: usize) -> RenderResult<u64> {
    let overflow = || {
        RenderError::new(
            RenderErrorKind::OutOfMemory,
            "native Vulkan surface dimensions exceed the staging budget range",
        )
    };
    let frame_bytes = u64::try_from(extent.width)
        .ok()
        .and_then(|width| {
            u64::try_from(extent.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(overflow)?;
    // The device divides the budget between reusable slots. Each slot must hold a complete
    // upload: a surface-sized RGBA image plus font atlases, previews, and scene geometry.
    // Allocate once at startup; never allocate or replace buffers while a slot is in flight.
    let per_slot = frame_bytes
        .checked_add(HEADROOM_BYTES_PER_SLOT)
        .ok_or_else(overflow)?
        .max(MIN_BYTES_PER_SLOT);
    per_slot
        .checked_mul(u64::try_from(frame_slots.max(1)).map_err(|_| overflow())?)
        .ok_or_else(overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_picker_first_upload_fits_every_reusable_slot() {
        let reported_uploads = [3_484_588, 3_304_996, 3_364_588];
        for slots in [1, 2, 3] {
            for scale in [1, 2] {
                let extent = SizeI {
                    width: 640 * scale,
                    height: 520 * scale,
                };
                let budget = surface_budget(extent, slots).unwrap();
                let per_slot = budget / slots as u64;
                for required in reported_uploads {
                    assert!(per_slot >= required);
                }
                assert!(
                    per_slot >= (extent.width * extent.height * 4) as u64 + HEADROOM_BYTES_PER_SLOT
                );
            }
        }
        // The former managed default was a total budget, not a per-slot budget.
        assert!(4 * 1024 * 1024 / 3 < reported_uploads[0]);
    }
    #[test]
    fn staging_budget_rejects_overflow_and_negative_extents() {
        assert!(
            surface_budget(
                SizeI {
                    width: i32::MAX,
                    height: i32::MAX
                },
                usize::MAX
            )
            .is_err()
        );
        assert!(
            surface_budget(
                SizeI {
                    width: -1,
                    height: 520
                },
                3
            )
            .is_err()
        );
    }
}

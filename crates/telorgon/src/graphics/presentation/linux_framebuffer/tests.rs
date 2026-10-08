use super::abi::{Bitfield, FixedInfo, VariableInfo};
use super::layout::Layout;

fn field(offset: u32, length: u32) -> Bitfield {
    Bitfield {
        offset,
        length,
        msb_right: 0,
    }
}

fn fixture() -> (FixedInfo, VariableInfo) {
    (
        FixedInfo {
            smem_len: 80,
            visual: 2,
            line_length: 20,
            ..FixedInfo::default()
        },
        VariableInfo {
            xres: 2,
            yres: 2,
            xres_virtual: 4,
            yres_virtual: 4,
            xoffset: 1,
            yoffset: 1,
            bits_per_pixel: 32,
            red: field(16, 8),
            green: field(8, 8),
            blue: field(0, 8),
            transparency: field(24, 8),
            ..VariableInfo::default()
        },
    )
}

#[test]
fn packed_conversion_respects_bitfields_stride_and_visible_pan_offsets() {
    let (fixed, variable) = fixture();
    let layout = Layout::read(&fixed, &variable).unwrap();
    assert_eq!(layout.visible_offset, 24);
    assert_eq!(layout.stride, 20);
    let mut row = [0; 8];
    layout.encode(&[10, 20, 30, 0, 255, 0, 0, 255], &mut row);
    assert_eq!(&row[..4], &0xff0a141eu32.to_ne_bytes());
    assert_eq!(&row[4..], &0xffff0000u32.to_ne_bytes());
    let mut mapping = [0x7f; 80];
    mapping[layout.visible_offset..layout.visible_offset + row.len()].copy_from_slice(&row);
    let next = layout.visible_offset + layout.stride;
    mapping[next..next + row.len()].copy_from_slice(&row);
    assert!(mapping[..24].iter().all(|byte| *byte == 0x7f));
    assert!(mapping[32..44].iter().all(|byte| *byte == 0x7f));
    assert!(mapping[52..].iter().all(|byte| *byte == 0x7f));
}

#[test]
fn rgb565_and_24_bit_pixels_encode_without_assuming_bgra() {
    let (mut fixed, mut variable) = fixture();
    variable.bits_per_pixel = 16;
    variable.red = field(11, 5);
    variable.green = field(5, 6);
    variable.blue = field(0, 5);
    variable.transparency = field(0, 0);
    fixed.line_length = 8;
    let layout = Layout::read(&fixed, &variable).unwrap();
    let mut pixel = [0; 2];
    layout.encode(&[255, 128, 0, 255], &mut pixel);
    assert_eq!(pixel, ((31u16 << 11) | (32u16 << 5)).to_ne_bytes());

    variable.bits_per_pixel = 24;
    variable.red = field(0, 8);
    variable.green = field(8, 8);
    variable.blue = field(16, 8);
    fixed.line_length = 12;
    let layout = Layout::read(&fixed, &variable).unwrap();
    let mut pixel = [0; 3];
    layout.encode(&[10, 20, 30, 255], &mut pixel);
    #[cfg(target_endian = "little")]
    assert_eq!(pixel, [10, 20, 30]);
    #[cfg(target_endian = "big")]
    assert_eq!(pixel, [30, 20, 10]);
}

#[test]
fn rejects_unsafe_geometry_palettes_and_overlapping_channels() {
    let (mut fixed, mut variable) = fixture();
    fixed.smem_len = 51;
    assert!(Layout::read(&fixed, &variable).is_err());
    fixed.smem_len = 80;
    variable.yoffset = 3;
    assert!(Layout::read(&fixed, &variable).is_err());
    variable.yoffset = 1;
    variable.green = variable.red;
    assert!(Layout::read(&fixed, &variable).is_err());
    variable.green = field(8, 8);
    fixed.visual = 4;
    assert!(Layout::read(&fixed, &variable).is_err());
    fixed.visual = 2;
    variable.vmode = 256;
    assert!(Layout::read(&fixed, &variable).is_err());
}

#[test]
fn ioctl_structures_match_the_native_fbdev_uapi_sizes() {
    assert_eq!(std::mem::size_of::<VariableInfo>(), 160);
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::size_of::<FixedInfo>(), 80);
    #[cfg(target_pointer_width = "32")]
    assert_eq!(std::mem::size_of::<FixedInfo>(), 68);
}

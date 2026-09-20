/// Packs straight-alpha sRGBA bytes into the ABI's numeric bit positions.
pub const fn pack_srgba8(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

pub const fn unpack_srgba8(value: u32) -> [u8; 4] {
    [
        value as u8,
        (value >> 8) as u8,
        (value >> 16) as u8,
        (value >> 24) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_words_are_host_endian_independent_numeric_values() {
        assert_eq!(pack_srgba8(0x12, 0x34, 0x56, 0x78), 0x7856_3412);
        assert_eq!(unpack_srgba8(0x7856_3412), [0x12, 0x34, 0x56, 0x78]);
    }
}

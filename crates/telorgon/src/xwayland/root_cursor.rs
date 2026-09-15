//! Bounded static root cursor pixels, independent of desktop/theme types.
use super::{Error, Result};

pub struct RootCursor {
    pub(crate) density: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) argb: Vec<u32>,
}
impl RootCursor {
    /// RGBA pixels at X11 density; the host maps surface size/hotspot back to logical units.
    pub fn new(
        width: i32,
        height: i32,
        x: i32,
        y: i32,
        rgba: &[u8],
        premultiplied: bool,
    ) -> Result<Self> {
        if !(1..=256).contains(&width)
            || !(1..=256).contains(&height)
            || !(0..width).contains(&x)
            || !(0..height).contains(&y)
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err(Error(
                "invalid X11 root cursor image or hotspot (maximum 256x256)".into(),
            ));
        }
        let argb = rgba
            .chunks_exact(4)
            .map(|p| {
                let a = u32::from(p[3]);
                let channel = |v: u8| {
                    if premultiplied {
                        u32::from(v).min(a)
                    } else {
                        (u32::from(v) * a + 127) / 255
                    }
                };
                (a << 24) | (channel(p[0]) << 16) | (channel(p[1]) << 8) | channel(p[2])
            })
            .collect();
        Ok(Self {
            density: 1,
            width: width as u16,
            height: height as u16,
            x: x as u16,
            y: y as u16,
            argb,
        })
    }
    pub(crate) fn bytes(&self, lsb: bool) -> Vec<u8> {
        self.argb
            .iter()
            .flat_map(|p| {
                if lsb {
                    p.to_le_bytes()
                } else {
                    p.to_be_bytes()
                }
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_alpha_hotspot_and_server_byte_order() {
        let image = RootCursor::new(1, 1, 0, 0, &[200, 100, 50, 128], false).unwrap();
        assert_eq!(image.argb, [0x80643219]);
        assert_eq!(image.bytes(true), [25, 50, 100, 128]);
        assert_eq!(image.bytes(false), [128, 100, 50, 25]);
        assert!(RootCursor::new(1, 1, 1, 0, &[0; 4], true).is_err());
        assert!(RootCursor::new(257, 1, 0, 0, &[], true).is_err());
        assert!(RootCursor::new(1, 1, 0, 0, &[], true).is_err());
    }
}

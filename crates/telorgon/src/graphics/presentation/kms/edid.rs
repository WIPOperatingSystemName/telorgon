/// Identity from a validated EDID 1.x base block. Extension timing data is left to DRM.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KmsMonitorIdentity {
    pub name: Option<String>,
    /// Three-letter EISA manufacturer identifier, not an expanded brand name.
    pub manufacturer: Option<String>,
    pub product_code: Option<u16>,
    pub serial_number: Option<String>,
}

impl KmsMonitorIdentity {
    pub fn from_edid(bytes: &[u8]) -> Option<Self> {
        let base = bytes.get(..128)?;
        if base[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
            || base[18] != 1
            || base.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0
        {
            return None;
        }
        let vendor = u16::from_be_bytes([base[8], base[9]]);
        let letters = [(vendor >> 10) & 31, (vendor >> 5) & 31, vendor & 31];
        let manufacturer = (vendor & 0x8000 == 0 && letters.iter().all(|v| (1..=26).contains(v)))
            .then(|| letters.iter().map(|v| char::from(b'A' + *v as u8 - 1)).collect());
        let serial = u32::from_le_bytes(base[12..16].try_into().unwrap());
        let mut identity = Self {
            manufacturer,
            product_code: Some(u16::from_le_bytes([base[10], base[11]])),
            serial_number: (serial != 0).then(|| serial.to_string()),
            name: None,
        };
        for descriptor in base[54..126].chunks_exact(18) {
            if descriptor[..3] != [0, 0, 0] || descriptor[4] != 0 {
                continue;
            }
            let value = descriptor[5..].split(|byte| *byte == 0 || *byte == b'\n').next()?;
            if !value.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
                continue;
            }
            let value = std::str::from_utf8(value).ok()?.trim();
            if value.is_empty() { continue; }
            match descriptor[3] {
                0xfc => identity.name = Some(value.to_owned()),
                0xff => identity.serial_number = Some(value.to_owned()),
                _ => {},
            }
        }
        Some(identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn checksum(bytes: &mut [u8; 128]) {
        bytes[127] = 0u8.wrapping_sub(bytes[..127].iter().fold(0u8, |s, b| s.wrapping_add(*b)));
    }
    fn fixture() -> [u8; 128] {
        let mut bytes = [0; 128];
        bytes[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        bytes[8..10].copy_from_slice(&0x10acu16.to_be_bytes()); // DEL
        bytes[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        bytes[12..16].copy_from_slice(&42u32.to_le_bytes());
        bytes[18] = 1;
        bytes[19] = 4;
        bytes[57] = 0xfc;
        bytes[59..72].copy_from_slice(b"Test Monitor\n");
        checksum(&mut bytes);
        bytes
    }
    #[test]
    fn decodes_identity_and_prefers_text_serial() {
        let mut bytes = fixture();
        let identity = KmsMonitorIdentity::from_edid(&bytes).unwrap();
        assert_eq!(identity.name.as_deref(), Some("Test Monitor"));
        assert_eq!(identity.manufacturer.as_deref(), Some("DEL"));
        assert_eq!(identity.product_code, Some(0x1234));
        assert_eq!(identity.serial_number.as_deref(), Some("42"));
        bytes[75] = 0xff;
        bytes[77..90].copy_from_slice(b"SERIAL-123\n  ");
        checksum(&mut bytes);
        assert_eq!(KmsMonitorIdentity::from_edid(&bytes).unwrap().serial_number.as_deref(), Some("SERIAL-123"));
    }
    #[test]
    fn rejects_corrupt_blocks_and_keeps_missing_fields_optional() {
        let mut bytes = fixture();
        for length in 0..128 { assert!(KmsMonitorIdentity::from_edid(&bytes[..length]).is_none()); }
        bytes[20] ^= 1;
        assert!(KmsMonitorIdentity::from_edid(&bytes).is_none());
        bytes[8..16].fill(0);
        bytes[54..126].fill(0);
        checksum(&mut bytes);
        let identity = KmsMonitorIdentity::from_edid(&bytes).unwrap();
        assert_eq!(identity.name, None);
        assert_eq!(identity.manufacturer, None);
        assert_eq!(identity.serial_number, None);
        bytes[0] = 1;
        checksum(&mut bytes);
        assert!(KmsMonitorIdentity::from_edid(&bytes).is_none());
    }
}

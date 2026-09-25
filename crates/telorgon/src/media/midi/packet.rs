use crate::integrations::pipewire::MediaError;
pub const MAX_SYSEX_BYTES: usize = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiRepresentation {
    Midi1,
    Ump,
}
/// Inline owned message; no allocation on queue transfer or cloning. UMP bytes use host
/// endianness per SPA_CONTROL_UMP; use ump_words for portable construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPacket {
    representation: MidiRepresentation,
    len: u16,
    bytes: [u8; MAX_SYSEX_BYTES],
}
impl MidiPacket {
    pub fn midi1(bytes: &[u8]) -> Result<Self, MediaError> {
        let Some(status) = bytes.first().copied() else {
            return Err(MediaError::InvalidArgument("empty MIDI message"));
        };
        if bytes.len() > MAX_SYSEX_BYTES {
            return Err(MediaError::ResourceLimit("MIDI SysEx bytes"));
        }
        let valid = match status {
            0x80..=0xbf | 0xe0..=0xef => bytes.len() == 3,
            0xc0..=0xdf | 0xf1 | 0xf3 => bytes.len() == 2,
            0xf2 => bytes.len() == 3,
            0xf6 | 0xf7 | 0xf8 | 0xfa | 0xfb | 0xfc | 0xfe | 0xff => bytes.len() == 1,
            0xf0 => bytes.len() >= 2 && bytes.last() == Some(&0xf7),
            _ => false,
        };
        let data_end = if status == 0xf0 {
            bytes.len().saturating_sub(1)
        } else {
            bytes.len()
        };
        if !valid || bytes[1..data_end].iter().any(|b| *b >= 0x80) {
            return Err(MediaError::InvalidArgument("complete MIDI 1 message"));
        }
        Ok(Self::copy(MidiRepresentation::Midi1, bytes))
    }
    pub fn ump_words(words: &[u32]) -> Result<Self, MediaError> {
        let Some(first) = words.first() else {
            return Err(MediaError::InvalidArgument("empty UMP"));
        };
        let count = match first >> 28 {
            0 | 1 | 2 => 1,
            3 | 4 => 2,
            5 | 0xd | 0xf => 4,
            _ => return Err(MediaError::Unsupported("reserved UMP message type")),
        };
        if words.len() != count {
            return Err(MediaError::InvalidArgument("UMP packet length"));
        }
        let mut bytes = [0; 16];
        for (word, output) in words.iter().zip(bytes.chunks_exact_mut(4)) {
            output.copy_from_slice(&word.to_ne_bytes());
        }
        Ok(Self::copy(MidiRepresentation::Ump, &bytes[..count * 4]))
    }
    pub(crate) fn from_bytes(
        representation: MidiRepresentation,
        bytes: &[u8],
    ) -> Result<Self, MediaError> {
        match representation {
            MidiRepresentation::Midi1 => Self::midi1(bytes),
            MidiRepresentation::Ump => {
                if bytes.is_empty() || bytes.len() > 16 || bytes.len() % 4 != 0 {
                    return Err(MediaError::InvalidArgument("UMP bytes"));
                }
                let mut words = [0u32; 4];
                for (word, bytes) in words.iter_mut().zip(bytes.chunks_exact(4)) {
                    *word = u32::from_ne_bytes(bytes.try_into().unwrap());
                }
                Self::ump_words(&words[..bytes.len() / 4])
            }
        }
    }
    fn copy(representation: MidiRepresentation, bytes: &[u8]) -> Self {
        let mut result = Self {
            representation,
            len: bytes.len() as u16,
            bytes: [0; MAX_SYSEX_BYTES],
        };
        result.bytes[..bytes.len()].copy_from_slice(bytes);
        result
    }
    pub fn representation(&self) -> MidiRepresentation {
        self.representation
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_midi_representations_and_sysex_limits() {
        assert!(MidiPacket::midi1(&[0x90, 60, 127]).is_ok());
        assert!(MidiPacket::midi1(&[60, 127]).is_err());
        assert!(MidiPacket::midi1(&[0x90, 60, 255]).is_err());
        let mut sysex = vec![0; MAX_SYSEX_BYTES];
        sysex[0] = 0xf0;
        *sysex.last_mut().unwrap() = 0xf7;
        assert_eq!(MidiPacket::midi1(&sysex).unwrap().bytes(), sysex);
        sysex.push(0xf7);
        assert!(MidiPacket::midi1(&sysex).is_err());
        for words in [
            &[0x20903c7f][..],
            &[0x40903c00, 0xffff0000][..],
            &[0x50000000, 0, 0, 0][..],
        ] {
            let packet = MidiPacket::ump_words(words).unwrap();
            assert_eq!(packet.bytes().len(), words.len() * 4);
            assert_eq!(
                MidiPacket::from_bytes(MidiRepresentation::Ump, packet.bytes()).unwrap(),
                packet
            );
        }
        assert!(MidiPacket::ump_words(&[0x40903c00]).is_err());
        assert!(MidiPacket::ump_words(&[0x60000000]).is_err());
    }
}

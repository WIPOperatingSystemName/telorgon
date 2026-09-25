//! Checked SPA sequence parsing/serialization, no native casts or allocation.
use super::*;
use pipewire::spa::sys;
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}
pub(crate) fn read_sequence(
    bytes: &[u8],
    clock: MidiTime,
    frames: u64,
    max_events: usize,
    mut event: impl FnMut(MidiEvent),
) -> Result<(), MediaError> {
    let invalid = || MediaError::InvalidArgument("MIDI SPA sequence");
    let size = u32_at(bytes, 0).ok_or_else(invalid)? as usize;
    if size < 8
        || u32_at(bytes, 4) != Some(sys::SPA_TYPE_Sequence)
        || size > bytes.len().saturating_sub(8)
        || u32_at(bytes, 8) != Some(0)
    {
        return Err(invalid());
    }
    let end = 8 + size;
    let mut offset = 16;
    let mut previous = 0;
    let mut count = 0;
    while offset < end {
        if count >= max_events {
            return Err(MediaError::ResourceLimit("MIDI cycle events"));
        }
        count += 1;
        let position = u32_at(bytes, offset).ok_or_else(invalid)? as u64;
        let kind = u32_at(bytes, offset + 4).ok_or_else(invalid)?;
        let len = u32_at(bytes, offset + 8).ok_or_else(invalid)? as usize;
        if position >= frames
            || position < previous
            || u32_at(bytes, offset + 12) != Some(sys::SPA_TYPE_Bytes)
        {
            return Err(invalid());
        }
        let finish = offset
            .checked_add(16)
            .and_then(|v| v.checked_add(len))
            .filter(|v| *v <= end)
            .ok_or_else(invalid)?;
        let next = finish
            .checked_add((8 - len % 8) % 8)
            .filter(|v| *v <= end)
            .ok_or_else(invalid)?;
        let representation = match kind {
            sys::SPA_CONTROL_Midi => MidiRepresentation::Midi1,
            sys::SPA_CONTROL_UMP => MidiRepresentation::Ump,
            _ => return Err(MediaError::Unsupported("MIDI control representation")),
        };
        let packet = MidiPacket::from_bytes(representation, &bytes[offset + 16..finish])?;
        let mut time = clock;
        if clock.rate_num == 0 || clock.rate_denom == 0 {
            return Err(invalid());
        }
        time.position = time.position.checked_add(position).ok_or_else(invalid)?;
        let delta = u64::try_from(
            position as u128 * clock.rate_num as u128 * 1_000_000_000 / clock.rate_denom as u128,
        )
        .map_err(|_| invalid())?;
        time.monotonic_ns = time.monotonic_ns.checked_add(delta).ok_or_else(invalid)?;
        event(MidiEvent { time, packet });
        previous = position;
        offset = next;
    }
    if offset != end {
        return Err(invalid());
    }
    Ok(())
}
/// Calls next with the remaining capacity (including the 16-byte control header). The
/// caller retains events whose padded packet size does not fit, and future events. Returns bytes initialized, and never splits one owned MIDI event.
pub(crate) fn write_sequence(
    bytes: &mut [u8],
    mut next: impl FnMut(usize) -> Option<(u32, MidiPacket)>,
) -> usize {
    if bytes.len() < 16 {
        return 0;
    }
    bytes[..16].fill(0);
    put(bytes, 4, sys::SPA_TYPE_Sequence);
    let mut offset = 16;
    while bytes.len() - offset >= 24 {
        let Some((position, packet)) = next(bytes.len() - offset) else {
            break;
        };
        let data = packet.bytes();
        let length = 16 + data.len().div_ceil(8) * 8;
        if length > bytes.len() - offset {
            break;
        }
        bytes[offset..offset + length].fill(0);
        put(bytes, offset, position);
        put(
            bytes,
            offset + 4,
            match packet.representation() {
                MidiRepresentation::Midi1 => sys::SPA_CONTROL_Midi,
                MidiRepresentation::Ump => sys::SPA_CONTROL_UMP,
            },
        );
        put(bytes, offset + 8, data.len() as u32);
        put(bytes, offset + 12, sys::SPA_TYPE_Bytes);
        bytes[offset + 16..offset + 16 + data.len()].copy_from_slice(data);
        offset += length;
    }
    put(bytes, 0, (offset - 8) as u32);
    offset
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_preserves_offsets_and_rejects_out_of_cycle_and_truncation() {
        let packet = MidiPacket::midi1(&[0x90, 60, 1]).unwrap();
        let mut event = Some((17, packet.clone()));
        let mut bytes = [0; 128];
        let len = write_sequence(&mut bytes, |_| event.take());
        let clock = MidiTime {
            clock_id: 4,
            position: 100,
            rate_num: 1,
            rate_denom: 48000,
            ..Default::default()
        };
        let mut actual = None;
        read_sequence(&bytes[..len], clock, 128, 8, |e| actual = Some(e)).unwrap();
        let actual = actual.unwrap();
        assert_eq!(actual.time.position, 117);
        assert_eq!(actual.packet, packet);
        assert!(read_sequence(&bytes[..len], clock, 16, 8, |_| {}).is_err());
        for n in 0..len {
            assert!(read_sequence(&bytes[..n], clock, 128, 8, |_| {}).is_err());
        }
    }
    #[test]
    fn parser_work_limit_preserves_prefix_and_rejects_malformed_padding() {
        let mut messages = 0;
        let mut bytes = [0; 128];
        let size = write_sequence(&mut bytes, |_| {
            messages += 1;
            (messages <= 3).then(|| (0, MidiPacket::midi1(&[0xf8]).unwrap()))
        });
        let mut delivered = 0;
        assert_eq!(
            read_sequence(&bytes[..size], MidiTime::default(), 128, 1, |_| {
                delivered += 1
            }),
            Err(MediaError::ResourceLimit("MIDI cycle events"))
        );
        assert_eq!(delivered, 1);
        // POD size must include the event's alignment padding, not just its payload.
        put(&mut bytes, 0, 25);
        assert!(read_sequence(&bytes, MidiTime::default(), 128, 8, |_| {}).is_err());
    }
}

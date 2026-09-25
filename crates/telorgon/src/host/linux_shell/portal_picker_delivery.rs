//! Fair, bounded delivery: source metadata cannot starve thumbnail updates.
use crate::host::application::portal_wire as wire;
use crate::{ScreenCastPortalSnapshot, shell::capture::CaptureSource};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

#[derive(Default)]
pub(super) struct PickerDelivery {
    metadata: Option<serde_json::Value>,
    previews: BTreeMap<CaptureSource, (u64, u64)>,
}
impl PickerDelivery {
    pub fn write(
        &mut self,
        writer: &mut impl Write,
        snapshot: &ScreenCastPortalSnapshot,
    ) -> io::Result<()> {
        let metadata = wire::snapshot(snapshot);
        if self.metadata.as_ref() != Some(&metadata) {
            wire::write(writer, &metadata)?;
            self.metadata = Some(metadata);
        }
        self.previews
            .retain(|source, _| snapshot.previews.iter().any(|p| p.source == *source));
        let Some((request, _)) = &snapshot.pending else {
            return Ok(());
        };
        for preview in &snapshot.previews {
            let version = (preview.epoch, preview.revision);
            if self.previews.get(&preview.source) != Some(&version) {
                wire::write(writer, &wire::preview(*request, preview))?;
                self.previews.insert(preview.source, version);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PortalSourcePreview, shell::OutputId};
    #[test]
    fn metadata_and_busy_first_source_cannot_starve_other_previews() {
        let mut snapshot = ScreenCastPortalSnapshot {
            pending: Some((4, "Recorder".into())),
            source_types: 1,
            ..Default::default()
        };
        for id in 1..=6 {
            let source = CaptureSource::Output(OutputId::from_raw(id).unwrap());
            snapshot.sources.push((source, 1, format!("Monitor {id}")));
            snapshot.previews.push(PortalSourcePreview {
                source,
                epoch: 1,
                revision: id,
                width: 1,
                height: 1,
                pixels: vec![id as u8, 0, 0, 255].into(),
            });
        }
        let mut delivery = PickerDelivery::default();
        let mut bytes = Vec::new();
        delivery.write(&mut bytes, &snapshot).unwrap();
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 7);
        let mut input = std::io::Cursor::new(bytes);
        let mut received = wire::parse_snapshot(wire::read(&mut input).unwrap()).unwrap();
        for _ in 0..6 {
            received = wire::update(wire::read(&mut input).unwrap(), &received).unwrap();
        }
        assert_eq!(received.previews, snapshot.previews);
        // Every thumbnail changes, but metadata does not: deliver all six immediately.
        for preview in &mut snapshot.previews {
            preview.revision += 10;
        }
        let mut bytes = Vec::new();
        delivery.write(&mut bytes, &snapshot).unwrap();
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 6);
        let mut bytes = Vec::new();
        delivery.write(&mut bytes, &snapshot).unwrap();
        assert!(bytes.is_empty());
    }
}

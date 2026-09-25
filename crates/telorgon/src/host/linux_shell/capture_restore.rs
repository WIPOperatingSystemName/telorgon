//! Source matching uses host/kernel identities, never display labels or client window titles.
use crate::graphics::presentation::kms::{KmsConnector, KmsDevice, KmsPropertyObject, KmsTopology};
use sha2::{Digest, Sha256};
use std::{os::unix::ffi::OsStrExt, path::Path};

pub(super) struct Identities {
    instance: String,
    output: Option<String>,
    epoch: Option<u64>,
}
impl Identities {
    pub fn new(output: Option<String>) -> Result<Self, String> {
        Ok(Self {
            instance: crate::integrations::portals::new_restore_instance()?,
            output,
            epoch: None,
        })
    }
    pub fn observe_output(&mut self, epoch: Option<u64>) {
        if let Some(epoch) = epoch {
            if self.epoch.is_some_and(|old| old != epoch) {
                // The cached EDID no longer proves identity after remapping. A newly queried
                // host can establish another persistent output key; this owner cannot.
                self.output = None;
            }
            self.epoch = Some(epoch);
        }
    }
    pub fn resolve(
        &self,
        keys: &[crate::integrations::portals::RestoreSource],
        candidates: impl IntoIterator<Item = (crate::shell::capture::CaptureSource, u64)>,
    ) -> Option<Vec<(crate::shell::capture::CaptureSource, u64)>> {
        if keys.is_empty() || keys.len() > 8 {
            return None;
        }
        let candidates: Vec<_> = candidates.into_iter().collect();
        let mut selected = Vec::with_capacity(keys.len());
        for key in keys {
            let mut matches = candidates
                .iter()
                .copied()
                .filter(|&(source, epoch)| self.key(source, epoch).as_ref() == Some(key));
            let found = matches.next()?;
            if matches.next().is_some() || selected.contains(&found) {
                return None;
            }
            selected.push(found);
        }
        Some(selected)
    }
    pub fn key(
        &self,
        source: crate::shell::capture::CaptureSource,
        epoch: u64,
    ) -> Option<crate::integrations::portals::RestoreSource> {
        use crate::shell::{OutputId, capture::CaptureSource};
        let (kind, identity) = match source {
            CaptureSource::Output(id) if id == OutputId::MIN && self.epoch == Some(epoch) => (
                1,
                self.output
                    .clone()
                    .unwrap_or_else(|| format!("output-session-v1:{}:{epoch}", self.instance)),
            ),
            CaptureSource::Window(id) => (2, window_key(&self.instance, id, epoch)),
            CaptureSource::VirtualOutput(id) => (
                4,
                format!("virtual-session-v1:{}:{}:{epoch}", self.instance, id.get()),
            ),
            _ => return None,
        };
        Some(crate::integrations::portals::RestoreSource { kind, identity })
    }
}

pub(super) fn output_key(
    device: &KmsDevice,
    connector: &KmsConnector,
    drm_path: &Path,
) -> Option<String> {
    let properties =
        KmsTopology::object_properties(device, connector.id.get(), KmsPropertyObject::Connector)
            .ok()?;
    let blob = u32::try_from(properties.named("EDID")?.value).ok()?;
    let edid = device.read_property_blob(blob, 32 * 1024).ok()?;
    let card = drm_path.file_name()?;
    // card numbers can change between boots; the canonical kernel device location cannot
    // silently select another GPU merely because it acquired the old card number.
    let location =
        std::fs::canonicalize(Path::new("/sys/class/drm").join(card).join("device")).ok()?;
    identity(
        &edid,
        location.as_os_str().as_bytes(),
        connector.connector_type,
        connector.connector_type_id,
    )
}

fn identity(edid: &[u8], device: &[u8], connector_type: u32, connector: u32) -> Option<String> {
    if edid.len() < 128
        || edid.len() > 32 * 1024
        || edid.len() % 128 != 0
        || edid[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || edid[18] != 1
        || edid.len() != (usize::from(edid[126]) + 1) * 128
        || edid
            .chunks_exact(128)
            .any(|block| block.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0)
        || device.is_empty()
        || device.len() > 4096
    {
        return None;
    }
    let serial = u32::from_le_bytes(edid[12..16].try_into().unwrap());
    let text_serial = edid[54..126].chunks_exact(18).any(|descriptor| {
        descriptor[..5] == [0, 0, 0, 255, 0]
            && descriptor[5..]
                .iter()
                .all(|b| b.is_ascii_graphic() || matches!(b, b' ' | b'\n' | 0))
            && descriptor[5..].iter().any(u8::is_ascii_graphic)
    });
    if matches!(serial, 0 | u32::MAX) && !text_serial {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"telorgon-output-v1\0");
    hash.update((device.len() as u32).to_le_bytes());
    hash.update(device);
    hash.update(connector_type.to_le_bytes());
    hash.update(connector.to_le_bytes());
    hash.update(edid);
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!("output-v1:{digest}"))
}

pub(super) fn window_key(instance: &str, id: crate::shell::WindowId, epoch: u64) -> String {
    format!(
        "window-v1:{instance}:{}:{}:{epoch}",
        id.slot(),
        id.generation()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edid() -> Vec<u8> {
        let mut value = vec![0; 128];
        value[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        value[18] = 1;
        value[19] = 4;
        value[12] = 42;
        checksum(&mut value);
        value
    }
    fn checksum(value: &mut [u8]) {
        value[127] = 0u8.wrapping_sub(value[..127].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)));
    }
    #[test]
    fn restored_groups_require_complete_unique_current_source_matches() {
        use crate::shell::{OutputId, WindowId, capture::CaptureSource};
        use std::num::NonZeroU32;
        let output = CaptureSource::Output(OutputId::MIN);
        let window = CaptureSource::Window(WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));
        let ids = Identities {
            instance: "host-a".into(),
            output: Some("stable-output".into()),
            epoch: Some(1),
        };
        let keys = vec![ids.key(output, 1).unwrap(), ids.key(window, 2).unwrap()];
        let candidates = [(output, 1), (window, 2)];
        assert_eq!(ids.resolve(&keys, candidates), Some(candidates.to_vec()));
        assert!(ids.resolve(&keys, [(output, 1)]).is_none());
        assert!(ids.resolve(&keys, [(output, 1), (window, 3)]).is_none());
        assert!(
            ids.resolve(&keys, [(output, 1), (window, 2), (window, 2)])
                .is_none()
        );
        assert!(
            ids.resolve(&[keys[0].clone(), keys[0].clone()], candidates)
                .is_none()
        );
        let new_host = Identities {
            instance: "host-b".into(),
            output: Some("stable-output".into()),
            epoch: Some(1),
        };
        assert!(new_host.resolve(&keys, candidates).is_none());
        assert_eq!(
            new_host.resolve(&keys[..1], candidates),
            Some(vec![(output, 1)])
        );
    }

    #[test]
    fn live_keys_bind_epoch_and_drop_cached_edid_after_output_remapping() {
        use crate::shell::{OutputId, capture::CaptureSource};
        let source = CaptureSource::Output(OutputId::MIN);
        let mut ids = Identities {
            instance: "host-a".into(),
            output: Some("stable-output".into()),
            epoch: None,
        };
        assert!(ids.key(source, 1).is_none());
        ids.observe_output(Some(1));
        assert_eq!(ids.key(source, 1).unwrap().identity, "stable-output");
        assert!(ids.key(source, 2).is_none());
        ids.observe_output(Some(2));
        let replacement = ids.key(source, 2).unwrap();
        assert_eq!(replacement.identity, "output-session-v1:host-a:2");
        assert!(ids.key(source, 1).is_none());
    }

    #[test]
    fn monitor_keys_bind_display_device_and_connector() {
        let mut value = edid();
        let key = identity(&value, b"/sys/devices/pci/gpu", 10, 1).unwrap();
        assert_eq!(
            Some(key.clone()),
            identity(&value, b"/sys/devices/pci/gpu", 10, 1)
        );
        assert_ne!(
            Some(key.clone()),
            identity(&value, b"/sys/devices/pci/other", 10, 1)
        );
        assert_ne!(
            Some(key.clone()),
            identity(&value, b"/sys/devices/pci/gpu", 10, 2)
        );
        value[12] = 43;
        checksum(&mut value);
        assert_ne!(Some(key), identity(&value, b"/sys/devices/pci/gpu", 10, 1));
    }
    #[test]
    fn missing_serial_corruption_and_incomplete_extensions_require_new_consent() {
        let mut value = edid();
        value[12] = 0;
        checksum(&mut value);
        assert!(identity(&value, b"gpu", 1, 1).is_none());
        value[54..59].copy_from_slice(&[0, 0, 0, 255, 0]);
        value[59..62].copy_from_slice(b"ABC");
        checksum(&mut value);
        assert!(identity(&value, b"gpu", 1, 1).is_some());
        value[60] ^= 1;
        assert!(identity(&value, b"gpu", 1, 1).is_none());
        checksum(&mut value);
        value[126] = 1;
        checksum(&mut value);
        assert!(identity(&value, b"gpu", 1, 1).is_none());
    }
    #[test]
    fn windows_cannot_restore_across_host_instances_remaps_or_reused_slots() {
        use std::num::NonZeroU32;
        let window = |generation| {
            crate::shell::WindowId::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(generation).unwrap(),
            )
        };
        let original = window_key("instance-a", window(1), 1);
        assert_ne!(original, window_key("instance-b", window(1), 1));
        assert_ne!(original, window_key("instance-a", window(2), 1));
        assert_ne!(original, window_key("instance-a", window(1), 2));
    }
}

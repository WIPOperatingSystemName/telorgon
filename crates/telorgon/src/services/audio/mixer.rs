//! Mixer presentation model. Application metadata is a grouping hint, not an identity authority.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApplicationGroupId {
    Application(String),
    Process(u32),
    Stream(ObjectHandle),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamDirection {
    Playback,
    Recording,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MixerTarget {
    Node(ObjectHandle),
    Application(ApplicationGroupId, StreamDirection),
    DefaultOutput,
    DefaultInput,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteState {
    Muted,
    Unmuted,
    Mixed,
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MixerApplication {
    pub id: ApplicationGroupId,
    pub name: String,
    pub icon_name: Option<String>,
    pub playback: Vec<AudioNode>,
    pub recording: Vec<AudioNode>,
}
impl MixerApplication {
    pub fn streams(&self, direction: StreamDirection) -> &[AudioNode] {
        match direction {
            StreamDirection::Playback => &self.playback,
            StreamDirection::Recording => &self.recording,
        }
    }
}
pub fn node_volume(node: &AudioNode) -> Option<f32> {
    node.channel_volumes
        .iter()
        .copied()
        .max_by(|a, b| a.value().total_cmp(&b.value()))
        .or(node.volume)
        .map(Gain::as_ui)
}
pub fn group_volume(nodes: &[AudioNode]) -> Option<f32> {
    nodes.iter().filter_map(node_volume).max_by(f32::total_cmp)
}
pub fn group_mute(nodes: &[AudioNode]) -> MuteState {
    if nodes.is_empty() || nodes.iter().any(|n| n.mute.is_none()) {
        return MuteState::Unknown;
    }
    let count = nodes.iter().filter(|n| n.mute == Some(true)).count();
    if count == nodes.len() {
        MuteState::Muted
    } else if count == 0 {
        MuteState::Unmuted
    } else {
        MuteState::Mixed
    }
}
pub fn applications(nodes: &[AudioNode]) -> Vec<MixerApplication> {
    let mut groups = BTreeMap::new();
    for node in nodes {
        let direction = match node.media_class.as_str() {
            "Stream/Output/Audio" => StreamDirection::Playback,
            "Stream/Input/Audio" => StreamDirection::Recording,
            _ => continue,
        };
        let id = node
            .application_id
            .as_ref()
            .filter(|id| !id.is_empty())
            .map(|id| ApplicationGroupId::Application(id.clone()))
            .or_else(|| node.process_id.map(ApplicationGroupId::Process))
            .unwrap_or(ApplicationGroupId::Stream(node.handle));
        let group = groups
            .entry(id.clone())
            .or_insert_with(|| MixerApplication {
                id,
                name: node
                    .application_name
                    .clone()
                    .or(node.application_id.clone())
                    .unwrap_or_else(|| {
                        if node.description.is_empty() {
                            node.name.clone()
                        } else {
                            node.description.clone()
                        }
                    }),
                icon_name: node.application_icon.clone(),
                playback: Vec::new(),
                recording: Vec::new(),
            });
        match direction {
            StreamDirection::Playback => &mut group.playback,
            StreamDirection::Recording => &mut group.recording,
        }
        .push(node.clone());
    }
    let mut result: Vec<_> = groups.into_values().collect();
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    result
}
/// UI units preserve the relative audible levels. An all-silent group is raised uniformly.
pub fn relative_volumes(
    nodes: &[AudioNode],
    wanted: f32,
) -> Result<Vec<(ObjectHandle, f32)>, MediaError> {
    if !wanted.is_finite() || !(0.0..=1.0).contains(&wanted) {
        return Err(MediaError::InvalidArgument("mixer volume 0..1"));
    }
    let peak = group_volume(nodes).ok_or(MediaError::Unsupported("observed volume"))?;
    nodes
        .iter()
        .map(|node| {
            if !node.can_set_volume {
                return Err(MediaError::PermissionDenied);
            }
            let level = node_volume(node).ok_or(MediaError::Unsupported("observed volume"))?;
            Ok((
                node.handle,
                if peak > 0.0 {
                    wanted * level / peak
                } else {
                    wanted
                },
            ))
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn node(
        id: u32,
        application: Option<&str>,
        process: Option<u32>,
        level: f32,
    ) -> AudioNode {
        AudioNode {
            handle: ObjectHandle {
                epoch: 1,
                incarnation: 1,
                id,
            },
            name: format!("stream-{id}"),
            description: format!("Stream {id}"),
            media_class: "Stream/Output/Audio".into(),
            application_id: application.map(str::to_owned),
            process_id: process,
            process_binary: None,
            application_name: None,
            application_icon: None,
            volume: Some(Gain::ui(level).unwrap()),
            mute: Some(false),
            channel_volumes: vec![],
            channel_map: vec![],
            can_set_volume: true,
        }
    }
    #[test]
    fn grouping_keeps_unknown_streams_separate_and_recording_distinct() {
        let mut recording = node(2, Some("browser"), Some(22), 0.4);
        recording.media_class = "Stream/Input/Audio".into();
        let result = applications(&[
            node(1, Some("browser"), Some(21), 0.8),
            recording,
            node(3, None, None, 0.3),
            node(4, None, None, 0.2),
            node(5, None, Some(80), 0.5),
            node(6, None, Some(80), 0.2),
        ]);
        assert_eq!(result.len(), 4);
        let browser = result
            .iter()
            .find(|a| a.id == ApplicationGroupId::Application("browser".into()))
            .unwrap();
        assert_eq!(browser.playback.len(), 1);
        assert_eq!(browser.recording.len(), 1);
        assert_eq!(
            result
                .iter()
                .find(|a| a.id == ApplicationGroupId::Process(80))
                .unwrap()
                .playback
                .len(),
            2
        );
    }
    #[test]
    fn group_volume_preserves_ratio_and_handles_silence_and_mixed_mute() {
        let mut nodes = vec![
            node(1, Some("app"), None, 0.8),
            node(2, Some("app"), None, 0.4),
        ];
        let values = relative_volumes(&nodes, 0.6).unwrap();
        assert!((values[0].1 - 0.6).abs() < 0.0001);
        assert!((values[1].1 - 0.3).abs() < 0.0001);
        nodes[0].mute = Some(true);
        assert_eq!(group_mute(&nodes), MuteState::Mixed);
        nodes[1].mute = None;
        assert_eq!(group_mute(&nodes), MuteState::Unknown);
        for node in &mut nodes {
            node.volume = Some(Gain::SILENCE);
        }
        assert!(
            relative_volumes(&nodes, 0.5)
                .unwrap()
                .iter()
                .all(|(_, v)| *v == 0.5)
        );
        assert!(relative_volumes(&nodes, f32::NAN).is_err());
        nodes[0].can_set_volume = false;
        assert!(relative_volumes(&nodes, 0.5).is_err());
    }
}

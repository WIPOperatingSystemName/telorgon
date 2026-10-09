//! Wire snapshots preserve connection-scoped identity without exposing native handles.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AudioObjectId {
    pub epoch: u64,
    pub incarnation: u64,
    pub id: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioDirection {
    Input,
    Output,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioApplicationId {
    Application(String),
    Process(u32),
    Stream(AudioObjectId),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioTarget {
    Node {
        id: AudioObjectId,
    },
    Application {
        id: AudioApplicationId,
        direction: AudioDirection,
    },
    DefaultOutput,
    DefaultInput,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioMuteState {
    Muted,
    Unmuted,
    Mixed,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioConnectionState {
    Connecting,
    Ready,
    Failed,
    Stopping,
    #[default]
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioDevice {
    pub name: String,
    pub label: String,
    pub input: bool,
    pub is_default: bool,
    pub volume: Option<f32>,
    pub muted: Option<bool>,
    pub can_set_volume: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioChannelInfo {
    pub index: usize,
    pub position: Option<u32>,
    pub label: String,
    pub volume: Option<f32>,
    pub linear_gain: Option<f32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioNodeInfo {
    pub id: AudioObjectId,
    pub name: String,
    pub label: String,
    pub media_class: String,
    pub device: Option<AudioObjectId>,
    pub profile_device: Option<i32>,
    pub is_virtual: Option<bool>,
    pub application_id: Option<String>,
    pub application_name: Option<String>,
    pub application_icon: Option<String>,
    pub process_id: Option<u32>,
    pub process_binary: Option<String>,
    pub media_role: Option<String>,
    pub advertised_rate: Option<u32>,
    pub advertised_format: Option<String>,
    pub bluetooth_codec: Option<String>,
    pub volume: Option<f32>,
    pub muted: Option<bool>,
    pub channels: Vec<AudioChannelInfo>,
    pub balance: Option<f32>,
    pub can_set_volume: bool,
    pub can_set_mute: bool,
    pub can_set_balance: bool,
    pub can_set_channel_volumes: bool,
    pub destinations: Vec<AudioObjectId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChoice {
    pub index: i32,
    pub name: String,
    pub label: String,
    pub available: Option<bool>,
    pub priority: Option<i32>,
    pub direction: Option<AudioDirection>,
    pub device: Option<i32>,
    pub devices: Vec<i32>,
    pub profiles: Vec<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioActiveRoute {
    pub index: i32,
    pub device: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioHardwareDevice {
    pub id: AudioObjectId,
    pub name: String,
    pub label: String,
    pub profiles: Vec<AudioChoice>,
    pub routes: Vec<AudioChoice>,
    pub active_profile: Option<i32>,
    pub active_routes: Vec<AudioActiveRoute>,
    pub can_set_profile: bool,
    pub can_set_route: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioApplication {
    pub id: AudioApplicationId,
    pub name: String,
    pub icon_name: Option<String>,
    pub playback: Vec<AudioObjectId>,
    pub recording: Vec<AudioObjectId>,
    pub playback_volume: Option<f32>,
    pub recording_volume: Option<f32>,
    pub playback_mute: AudioMuteState,
    pub recording_mute: AudioMuteState,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioOperation {
    pub target: AudioTarget,
    pub pending: bool,
    pub preview_volume: Option<f32>,
    pub error: Option<String>,
    pub applied: usize,
    pub failed: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSnapshot {
    pub generation: u64,
    pub state: AudioConnectionState,
    pub error: Option<String>,
    pub default_output: Option<AudioObjectId>,
    pub default_input: Option<AudioObjectId>,
    pub nodes: Vec<AudioNodeInfo>,
    pub hardware_devices: Vec<AudioHardwareDevice>,
    pub applications: Vec<AudioApplication>,
    pub operations: Vec<AudioOperation>,
}

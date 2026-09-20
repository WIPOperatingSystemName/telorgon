#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneUpdateStats {
    pub epoch: u64,
    pub upload_bytes_queued: u64,
    pub descriptor_writes_queued: u32,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RenderStats {
    pub recorded: bool,
    pub epoch: u64,
    pub upload_bytes_recorded: u64,
    pub buffer_copies: u32,
    pub buffer_allocations: u32,
    pub descriptor_writes: u32,
    pub passes: u32,
    pub barriers: u32,
    pub batches: u32,
    pub draws: u32,
    pub dispatches: u32,
    pub damage_area: f32,
}

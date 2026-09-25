//! Completed query results are retained with the receipt, independently of slot reuse.
use super::CompletionPoint;
use std::time::Instant;

#[derive(Clone, Debug)]
pub(crate) struct GpuTiming {
    pub total_ns: u64,
    pub upload_ns: u64,
    pub stages: Vec<(&'static str, u64)>,
    pub stages_available: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SubmissionTiming {
    pub point: CompletionPoint,
    pub submit_started: Instant,
    pub submitted: Instant,
    pub gpu: Option<GpuTiming>,
}

pub(crate) type TimingResult = std::sync::Arc<std::sync::Mutex<Option<GpuTiming>>>;

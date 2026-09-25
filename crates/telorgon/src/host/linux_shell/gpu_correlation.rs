//! Late transition frames join CPU observations, device-local GPU durations, and
//! presentation metadata by receipt identity. No GPU/CPU clock subtraction is used.
use super::*;
use crate::graphics::renderers::vulkan::SubmissionTiming;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Default)]
pub(super) struct Details {
    pub shell_frame: u64,
    pub submission: Option<SubmissionTiming>,
    pub target_ns: Option<u64>,
    pub waits: [u64; 3],
    pub flip: Option<(u32, Option<u64>)>,
}
#[derive(Clone)]
pub(super) struct Completed {
    pub capture: bool,
    pub timing: SubmissionTiming,
    pub done: Instant,
}

pub(super) fn retain(history: &mut VecDeque<Completed>, sample: Completed) {
    history.push_back(sample);
    while history.len() > 32 {
        history.pop_front();
    }
}

pub(super) fn report(
    frame: &Frame,
    window: u32,
    transition: u64,
    gap: Duration,
    now: Instant,
    history: &VecDeque<Completed>,
) {
    let details = &frame.correlation;
    let Some(submission) = &details.submission else {
        eprintln!(
            "telorgon-gpu-frame: frame={} window={window} transition={transition} gap_ms={:.3} gpu=unavailable",
            details.shell_frame,
            ms(gap)
        );
        return;
    };
    let done = frame.worker_completed.unwrap_or(now);
    let submit_to_worker = done.saturating_duration_since(submission.submitted);
    let execution = submission
        .gpu
        .as_ref()
        .map(|gpu| Duration::from_nanos(gpu.total_ns));
    // This residual includes queueing, synchronization, query resolution and worker
    // observation delay. It is deliberately not advertised as pure GPU queue time.
    let residual = execution.map(|gpu| submit_to_worker.saturating_sub(gpu));
    eprintln!(
        "telorgon-gpu-frame: frame={} window={window} transition={transition} device={} gpu_frame={} timeline={} gap_ms={:.3} cpu_record_ms={:.3} cpu_submit_ms={:.3} gpu_total_ms={:?} gpu_upload_ms={:?} submit_to_worker_ms={:.3} unattributed_wait_ms={:?} worker_to_owner_ms={:.3} wait_commit_ms={:.3} wait_pacing_ms={:.3} wait_dispatch_ms={:.3} target_ns={:?} drm_flip={:?}",
        details.shell_frame,
        submission.point.device_id(),
        submission.point.frame_id(),
        submission.point.value(),
        ms(gap),
        ms(submission
            .submit_started
            .saturating_duration_since(frame.prepared)),
        ms(submission
            .submitted
            .saturating_duration_since(submission.submit_started)),
        execution.map(ms),
        submission
            .gpu
            .as_ref()
            .map(|gpu| gpu.upload_ns as f64 / 1e6),
        ms(submit_to_worker),
        residual.map(ms),
        ms(frame.ready.unwrap_or(now).saturating_duration_since(done)),
        details.waits[0] as f64 / 1e6,
        details.waits[1] as f64 / 1e6,
        details.waits[2] as f64 / 1e6,
        details.target_ns,
        details.flip
    );
    if let Some(gpu) = &submission.gpu {
        let mut stages = BTreeMap::<&str, u64>::new();
        for (name, ns) in &gpu.stages {
            *stages.entry(name).or_default() += *ns;
        }
        eprintln!(
            "telorgon-gpu-passes: frame={} gpu_frame={} available={} durations_ms={:?}",
            details.shell_frame,
            submission.point.frame_id(),
            gpu.stages_available,
            stages
                .into_iter()
                .map(|(name, ns)| (name, ns as f64 / 1e6))
                .collect::<Vec<_>>()
        );
    }
    let mut neighbors = history
        .iter()
        .filter(|sample| {
            sample.timing.point.device_id() == submission.point.device_id()
                && sample.timing.point.value() < submission.point.value()
        })
        .collect::<Vec<_>>();
    neighbors.sort_by_key(|sample| sample.timing.point.value());
    for sample in neighbors.into_iter().rev().take(3) {
        eprintln!(
            "telorgon-gpu-neighbor: frame={} kind={} gpu_frame={} timeline={} gpu_total_ms={:?} submit_to_worker_ms={:.3}",
            details.shell_frame,
            if sample.capture { "capture" } else { "display" },
            sample.timing.point.frame_id(),
            sample.timing.point.value(),
            sample
                .timing
                .gpu
                .as_ref()
                .map(|gpu| gpu.total_ns as f64 / 1e6),
            ms(sample
                .done
                .saturating_duration_since(sample.timing.submitted))
        );
    }
}

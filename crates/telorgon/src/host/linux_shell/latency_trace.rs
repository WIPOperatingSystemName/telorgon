//! Bounded, opt-in flight recorder. No file writes, locks, or per-event allocations during capture.
//! Times are owner-thread observations, never GPU timestamps or proof of input-to-photon latency.
use std::{
    cell::RefCell,
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    rc::Rc,
    time::Instant,
};

const CAPACITY: usize = 262_144;

#[derive(Clone, Copy)]
struct Event {
    name: &'static str,
    ts_us: u64,
    duration_us: Option<u64>,
    values: [u64; 4],
}

struct Recording {
    start: Instant,
    phase: Option<(&'static str, u64)>,
    events: VecDeque<Event>,
    dropped: u64,
    capacity: usize,
}

impl Recording {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            start: Instant::now(),
            phase: None,
            events: VecDeque::with_capacity(capacity),
            dropped: 0,
            capacity,
        }
    }

    fn now(&self) -> u64 {
        self.start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

    fn push(&mut self, event: Event) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }

    fn finish_phase(&mut self, now: u64) {
        if let Some((name, ts_us)) = self.phase.take() {
            self.push(Event {
                name,
                ts_us,
                duration_us: Some(now.saturating_sub(ts_us)),
                values: [0; 4],
            });
        }
    }

    fn write(&self, mut out: impl Write) -> io::Result<()> {
        writeln!(
            out,
            "{{\"schema\":1,\"clock\":\"owner_elapsed_us\",\"capacity\":{},\"dropped\":{},\"events\":{}}}",
            self.capacity,
            self.dropped,
            self.events.len()
        )?;
        for event in &self.events {
            // Names are static internal labels; never client-controlled strings.
            write!(
                out,
                "{{\"name\":\"{}\",\"ts_us\":{},\"duration_us\":",
                event.name, event.ts_us
            )?;
            match event.duration_us {
                Some(duration) => write!(out, "{duration}")?,
                None => write!(out, "null")?,
            }
            writeln!(out, ",\"values\":{:?}}}", event.values)?;
        }
        out.flush()
    }
}

#[derive(Default)]
pub(in crate::host::linux_shell) struct LatencyTrace {
    recording: Option<Rc<RefCell<Recording>>>,
    output: Option<File>,
}

impl LatencyTrace {
    pub(super) fn from_env() -> io::Result<Self> {
        let Some(path) = std::env::var_os("TELORGON_LATENCY_TRACE") else {
            return Ok(Self::default());
        };
        // Fail visibly rather than silently running an experiment without its evidence. Reserve a
        // new private file now; existing files/symlinks are never truncated or followed.
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        eprintln!(
            "telorgon-latency: recording up to {CAPACITY} events in memory; writing on normal exit"
        );
        Ok(Self {
            recording: Some(Rc::new(RefCell::new(Recording::new(CAPACITY)))),
            output: Some(output),
        })
    }

    pub(super) fn protocol_observer(
        &self,
    ) -> Option<crate::integrations::wayland::compositor::TimingObserver> {
        use crate::integrations::wayland::compositor::TimingEvent;
        let recording = self.recording.clone()?;
        Some(Box::new(move |event| {
            let (name, values) = match event {
                TimingEvent::CommitRequested {
                    surface,
                    current_revision,
                    attaches_buffer,
                    callbacks,
                } => (
                    "client_commit_request",
                    [
                        u64::from(surface),
                        current_revision,
                        u64::from(attaches_buffer),
                        callbacks as u64,
                    ],
                ),
                TimingEvent::CallbacksQueued {
                    surface,
                    revision,
                    count,
                    presented,
                } => (
                    "frame_callbacks_queued",
                    [
                        u64::from(surface),
                        revision,
                        count as u64,
                        u64::from(presented),
                    ],
                ),
            };
            let mut recording = recording.borrow_mut();
            let ts_us = recording.now();
            recording.push(Event {
                name,
                values,
                ts_us,
                duration_us: None,
            });
        }))
    }

    pub(super) fn enabled(&self) -> bool {
        self.recording.is_some()
    }

    pub(in crate::host::linux_shell) fn phase(&mut self, name: &'static str) {
        if let Some(recording) = &mut self.recording {
            let mut recording = recording.borrow_mut();
            let now = recording.now();
            recording.finish_phase(now);
            recording.phase = Some((name, now));
        }
    }

    pub(super) fn event(&mut self, name: &'static str, values: [u64; 4]) {
        if let Some(recording) = &mut self.recording {
            let mut recording = recording.borrow_mut();
            let ts_us = recording.now();
            recording.push(Event {
                name,
                ts_us,
                duration_us: None,
                values,
            });
        }
    }
}

impl Drop for LatencyTrace {
    fn drop(&mut self) {
        if let (Some(recording), Some(output)) = (&mut self.recording, self.output.take()) {
            let mut recording = recording.borrow_mut();
            let now = recording.now();
            recording.finish_phase(now);
            if let Err(error) = recording.write(BufWriter::new(output)) {
                eprintln!("telorgon-latency: capture write failed: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_retains_newest_events_and_reports_loss() {
        let mut recording = Recording::new(2);
        for ts_us in 0..5 {
            recording.push(Event {
                name: "input_flush",
                ts_us,
                duration_us: None,
                values: [0; 4],
            });
        }
        assert_eq!(recording.dropped, 3);
        assert_eq!(
            recording
                .events
                .iter()
                .map(|event| event.ts_us)
                .collect::<Vec<_>>(),
            [3, 4]
        );
        let mut out = Vec::new();
        recording.write(&mut out).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.lines().next().unwrap().contains("\"dropped\":3"));
        assert_eq!(output.lines().count(), 3);
    }

    #[test]
    fn phases_are_disjoint_even_when_instants_arrive_between_them() {
        let mut recording = Recording::new(8);
        recording.phase = Some(("input", 20));
        recording.push(Event {
            name: "input_flush",
            ts_us: 25,
            duration_us: None,
            values: [1, 1, 5, 6],
        });
        recording.finish_phase(30);
        recording.finish_phase(40);
        assert_eq!(recording.events.len(), 2);
        assert_eq!(recording.events[1].ts_us, 20);
        assert_eq!(recording.events[1].duration_us, Some(10));
        assert!(recording.phase.is_none());
    }

    #[test]
    fn disabled_recorder_does_not_allocate_or_record() {
        let mut trace = LatencyTrace::default();
        trace.phase("input");
        trace.event("input_flush", [0; 4]);
        assert!(!trace.enabled());
    }
}

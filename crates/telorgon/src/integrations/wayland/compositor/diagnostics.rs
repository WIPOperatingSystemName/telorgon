use std::{
    collections::BTreeMap,
    fmt,
    sync::{Mutex, OnceLock},
    time::Instant,
};

#[derive(Default)]
struct Samples {
    events: BTreeMap<(u32, &'static str), (usize, String)>,
    total: usize,
}

fn limit(name: &str, default: usize, maximum: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
        .min(maximum)
}

// Opt-in metadata only: no pixels, window titles, audio, or input contents.
// Bound both repeated events and total output, including for long sessions.
pub(crate) fn event(surface: u32, stage: &'static str, details: fmt::Arguments<'_>) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    static START: OnceLock<Instant> = OnceLock::new();
    static SAMPLES: OnceLock<Mutex<Samples>> = OnceLock::new();
    static LIMITS: OnceLock<(usize, usize)> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("TELORGON_SURFACE_DEBUG").as_deref() == Ok("1")) {
        return;
    }
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    let (per_stage, total) = *LIMITS.get_or_init(|| {
        (
            limit("TELORGON_SURFACE_DEBUG_SAMPLES", 24, 4096),
            limit("TELORGON_SURFACE_DEBUG_TOTAL", 4096, 262144),
        )
    });
    let Ok(mut samples) = SAMPLES.get_or_init(Default::default).lock() else {
        return;
    };
    if samples.total >= total {
        return;
    }
    let sample = samples.events.entry((surface, stage)).or_default();
    if sample.0 >= per_stage {
        return;
    }
    let details = details.to_string();
    if sample.1 == details && sample.0 != 0 {
        return;
    }
    sample.0 += 1;
    sample.1 = details;
    eprintln!(
        "telorgon-surface +{elapsed}ms id={surface} {stage}: {}",
        sample.1
    );
    if sample.0 == per_stage {
        eprintln!("telorgon-surface id={surface} {stage}: sample limit reached");
    }
    samples.total += 1;
    if samples.total == total {
        eprintln!("telorgon-surface: total sample limit reached");
    }
}

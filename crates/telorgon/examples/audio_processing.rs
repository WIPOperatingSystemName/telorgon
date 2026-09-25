//! Offline DSP example: --no-default-features --features audio-linux --example audio_processing -- NEW.wav
//! Generates a synthetic stereo signal, timed gain, mono channel mixing and 48→44.1 kHz
//! resampling. Append --drift-ppm 200 to demonstrate correction for a synthetic fast
//! source clock. No PipeWire connection, playback or recording is opened.
#[cfg(target_os = "linux")]
#[path = "support/pcm_wave.rs"]
mod pcm_wave;
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use telorgon::media::audio::{AudioClockSnapshot, AudioTiming, SincResampler, dsp::*};
    let path = std::env::args_os()
        .nth(1)
        .ok_or("provide a new output WAV path")?;
    let options: Vec<String> = std::env::args().skip(2).collect();
    let drift_ppm: Option<f64> = match options.as_slice() {
        [] => None,
        [flag, value] if flag == "--drift-ppm" => {
            let value: f64 = value.parse()?;
            if !value.is_finite() || value.abs() > 500.0 {
                return Err("synthetic drift must be within ±500 ppm".into());
            }
            Some(value)
        }
        _ => return Err("expected optional --drift-ppm NUMBER".into()),
    };
    let mut wave = pcm_wave::Wave::create(std::path::Path::new(&path), 44100, 1)?;
    let mixer = ChannelMixer::new(2, 1, vec![0.5, 0.5])?;
    let (mut automation, mut gain) = TimedGain::new(0.2, 8)?;
    automation.schedule(GainUpdate {
        frame: 48000,
        gain: 0.5,
    })?;
    automation.schedule(GainUpdate {
        frame: 96000,
        gain: 0.1,
    })?;
    let mut resampler = SincResampler::new(48000, 44100, 1, 4096)?;
    let mut stereo = vec![0.0; 512 * 2];
    let mut mono = vec![0.0; 512];
    let mut output = vec![0.0; 1024];
    for start in (0..144000usize).step_by(512) {
        let frames = (144000 - start).min(512);
        if let Some(ppm) = drift_ppm {
            let monotonic_ns = (start as f64 * 1e9 / (48000.0 * (1.0 + ppm / 1e6))) as i64;
            let input = AudioClockSnapshot {
                timing: AudioTiming {
                    graph_ticks: start as u64,
                    rate_num: 1,
                    rate_denom: 48000,
                    monotonic_ns,
                    format_generation: 1,
                    ..Default::default()
                },
                sample_rate: 48000,
                quantum_frames: frames as u32,
                discontinuities: 0,
            };
            let output_clock = AudioClockSnapshot {
                timing: AudioTiming {
                    graph_ticks: monotonic_ns as u64,
                    rate_num: 1,
                    rate_denom: 1_000_000_000,
                    monotonic_ns,
                    format_generation: 1,
                    ..Default::default()
                },
                sample_rate: 44100,
                quantum_frames: frames as u32,
                discontinuities: 0,
            };
            let adjustment = resampler.synchronize_clocks(input, output_clock, monotonic_ns)?;
            if start + frames == 144000 {
                eprintln!(
                    "Estimated relative drift: {:.2} ppm",
                    adjustment.correction_ppm
                );
            }
        }
        for (index, frame) in stereo[..frames * 2].chunks_exact_mut(2).enumerate() {
            let time = (start + index) as f32 / 48000.0;
            frame[0] = (time * 440.0 * std::f32::consts::TAU).sin();
            frame[1] = (time * 660.0 * std::f32::consts::TAU).sin();
        }
        gain.process(&mut stereo[..frames * 2], 2)?;
        mixer.process(&stereo[..frames * 2], &mut mono[..frames])?;
        let mut consumed = 0;
        while consumed < frames {
            let result = resampler.process(&mono[consumed..frames], &mut output)?;
            consumed += result.consumed_frames;
            wave.write(&output[..result.produced_frames])?;
            if result.consumed_frames == 0 && result.produced_frames == 0 {
                return Err("resampler made no progress".into());
            }
        }
    }
    // Drain the finite filter tail with explicit silence, then any buffered output.
    let result = resampler.process(&[0.0; 32], &mut output)?;
    wave.write(&output[..result.produced_frames])?;
    loop {
        let result = resampler.process(&[], &mut output)?;
        if result.produced_frames == 0 {
            break;
        }
        wave.write(&output[..result.produced_frames])?;
    }
    wave.finish()?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires the Linux audio feature.");
}

//! Parent-owned readings; this component never opens an audio stream.
use crate::{authoring::compose::*, media::audio::AudioLevels};

const PAGE: usize = 4;

/// Peak and RMS amplitudes relative to digital full scale. The parent polls an
/// `AudioMeter` and supplies `None` when readings become unavailable or stale.
/// Peaks at or above full scale indicate potential clipping, not device clipping.
#[crate::component(no_default)]
pub struct AudioLevelMeter {
    #[input]
    levels: Option<AudioLevels>,
    #[state]
    page: usize,
}
impl AudioLevelMeter {
    pub fn new(levels: Option<AudioLevels>) -> Self {
        Self { levels, page: 0 }
    }
}
fn decibels(amplitude: f32) -> String {
    if !amplitude.is_finite() || amplitude < 0.0 {
        "unavailable".into()
    } else if amplitude == 0.0 {
        "−∞ dBFS".into()
    } else {
        format!("{:.1} dBFS", 20.0 * amplitude.log10())
    }
}
impl Component for AudioLevelMeter {
    fn view(&self) -> impl View {
        let mut content = column().gap(6.0).child(text("Audio levels"));
        let Some(levels) = self.levels.filter(|levels| levels.channels > 0) else {
            return content.child(text("Levels unavailable"));
        };
        let channels = levels.channels.min(levels.levels.len());
        let last = channels.saturating_sub(1) / PAGE;
        let page = self.page.min(last);
        for index in page * PAGE..((page + 1) * PAGE).min(channels) {
            let level = levels.levels[index];
            content = content.child(text(format!(
                "Channel {} · peak {} · RMS {}{}",
                index + 1,
                decibels(level.peak),
                decibels(level.rms),
                if level.peak.is_finite() && level.peak >= 1.0 {
                    " · FULL SCALE"
                } else {
                    ""
                }
            )));
        }
        if last > 0 {
            content = content.child(
                row()
                    .gap(8.0)
                    .height(32.0)
                    .child(
                        button("Previous channels")
                            .enabled(page > 0)
                            .on_press(move |this: &mut Self| this.page = page.saturating_sub(1)),
                    )
                    .child(text(format!("{}/{}", page + 1, last + 1)))
                    .child(
                        button("Next channels")
                            .enabled(page < last)
                            .on_press(move |this: &mut Self| this.page = (page + 1).min(last)),
                    ),
            );
        }
        content
    }
}

use super::{controller::Shared, *};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenBrightnessKeyHandling {
    #[default]
    ShellAdjusts,
    FirmwareAdjusts,
    Disabled,
}
#[derive(Clone, Copy, Debug)]
pub struct ScreenBrightnessKeyConfig {
    pub handling: ScreenBrightnessKeyHandling,
    pub step: ScreenBrightnessDelta,
    pub repeat_delay: Duration,
    pub repeat_interval: Duration,
}
impl Default for ScreenBrightnessKeyConfig {
    fn default() -> Self {
        Self {
            handling: Default::default(),
            step: ScreenBrightnessDelta::basis_points(500).unwrap(),
            repeat_delay: Duration::from_millis(350),
            repeat_interval: Duration::from_millis(100),
        }
    }
}
impl ScreenBrightnessKeyConfig {
    pub(crate) fn validate(&self) -> Result<(), ScreenBrightnessError> {
        if self.step.as_basis_points() <= 0
            || self.repeat_delay < Duration::from_millis(100)
            || self.repeat_delay > Duration::from_secs(5)
            || self.repeat_interval < Duration::from_millis(50)
            || self.repeat_interval > Duration::from_secs(2)
        {
            return Err(ScreenBrightnessError::InvalidConfig(
                "brightness key step or repeat timing",
            ));
        }
        Ok(())
    }
}
pub(crate) struct HeldKey {
    pub(super) authority: u64,
    device: ScreenBrightnessDeviceHandle,
    epoch: u64,
    delta: ScreenBrightnessDelta,
    next: Instant,
    interval: Duration,
}
impl ScreenBrightnessHandle {
    pub(crate) fn key_press(
        &self,
        target: ScreenBrightnessTarget,
        keycode: u32,
        up: bool,
        config: ScreenBrightnessKeyConfig,
    ) -> Result<(), ScreenBrightnessError> {
        match config.handling {
            ScreenBrightnessKeyHandling::Disabled => return Ok(()),
            ScreenBrightnessKeyHandling::FirmwareAdjusts => {
                self.shared.lock().refresh_requested = true;
                self.shared.wake.notify_all();
                return Ok(());
            }
            ScreenBrightnessKeyHandling::ShellAdjusts => {}
        }
        let delta = ScreenBrightnessDelta::basis_points(if up {
            config.step.as_basis_points()
        } else {
            -config.step.as_basis_points()
        })?;
        let request = self.enqueue(
            target,
            ScreenBrightnessAction::Adjust(delta),
            ScreenBrightnessWriteMode::Ordered,
            // A tap remains admitted after release; only queued held repeats are cancelled.
            None,
            None,
        )?;
        let mut state = self.shared.lock();
        if state.epoch == request.epoch
            && !state.stopped
            && state.host.active
            && !state.host.locked
            && state.keys.len() < 32
        {
            state.keys.insert(
                keycode,
                HeldKey {
                    authority: self.authority,
                    device: request.device,
                    epoch: request.epoch,
                    delta,
                    next: Instant::now() + config.repeat_delay,
                    interval: config.repeat_interval,
                },
            );
        }
        Ok(())
    }
}
impl ScreenBrightnessController {
    pub(crate) fn release_key(&self, keycode: u32) {
        let cancelled = {
            let mut state = self.shared.lock();
            state.keys.remove(&keycode);
            let mut cancelled = Vec::new();
            state.queue.retain(|c| {
                if c.key == Some(keycode) {
                    cancelled.push(c.completion.clone());
                    false
                } else {
                    true
                }
            });
            cancelled
        };
        for completion in cancelled {
            completion.finish(ScreenBrightnessOutcome::Cancelled);
        }
        self.shared.wake.notify_all();
    }
    pub(crate) fn cancel_keys(&self) {
        let keys = self.shared.lock().keys.keys().copied().collect::<Vec<_>>();
        for key in keys {
            self.release_key(key);
        }
    }
}
pub(super) fn repeat(shared: &Arc<Shared>) {
    let due = {
        let mut state = shared.lock();
        if state.stopped || !state.host.active || state.host.locked {
            state.keys.clear();
            return;
        }
        let epoch = state.epoch;
        let mut due = Vec::new();
        for (&keycode, key) in &mut state.keys {
            if key.epoch == epoch && Instant::now() >= key.next {
                key.next = Instant::now() + key.interval;
                due.push((keycode, key.authority, key.device, key.epoch, key.delta));
            }
        }
        due
    };
    for (keycode, authority, device, epoch, delta) in due {
        let handle = ScreenBrightnessHandle {
            shared: shared.clone(),
            authority,
        };
        let _ = handle.enqueue(
            ScreenBrightnessTarget::Device(device),
            ScreenBrightnessAction::Adjust(delta),
            ScreenBrightnessWriteMode::Ordered,
            Some(keycode),
            Some(epoch),
        );
    }
}

use core::cell::Cell;
use core::ffi::c_void;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use r_efi::{efi, protocols::timestamp};

use super::{UefiContext, UefiError, UefiResult, check};

const TIMER_PERIOD_NANOS: u64 = 10_000_000;

struct TimerCounter {
    ticks: AtomicU64,
}

enum ClockSource {
    Timestamp {
        protocol: NonNull<timestamp::Protocol>,
        frequency: u64,
        end_value: u64,
        last_counter: Cell<u64>,
        elapsed_ticks: Cell<u128>,
    },
    Timer {
        event: efi::Event,
        counter: NonNull<TimerCounter>,
    },
}

/// One zero-based monotonic domain scoped to active firmware services.
pub struct UefiClock<'a> {
    context: &'a UefiContext,
    source: ClockSource,
    last_nanos: Cell<u64>,
}

unsafe extern "efiapi" fn timer_tick(_event: efi::Event, context: *mut c_void) {
    let counter = unsafe { &*context.cast::<TimerCounter>() };
    let _ = counter
        .ticks
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
}

impl<'a> UefiClock<'a> {
    pub fn new(context: &'a UefiContext) -> UefiResult<Self> {
        context.ensure_active()?;
        if let Ok(protocol) =
            context.locate_protocol::<timestamp::Protocol>(timestamp::PROTOCOL_GUID)
        {
            let mut properties = timestamp::Properties {
                frequency: 0,
                end_value: 0,
            };
            let status = unsafe { (protocol.as_ref().get_properties)(&mut properties) };
            if !status.is_error() && properties.frequency != 0 && properties.end_value != 0 {
                let start = unsafe { (protocol.as_ref().get_timestamp)() };
                if start <= properties.end_value {
                    return Ok(Self {
                        context,
                        source: ClockSource::Timestamp {
                            protocol,
                            frequency: properties.frequency,
                            end_value: properties.end_value,
                            last_counter: Cell::new(start),
                            elapsed_ticks: Cell::new(0),
                        },
                        last_nanos: Cell::new(0),
                    });
                }
            }
        }
        let services = context.services()?;
        let mut storage = ptr::null_mut();
        check("allocate firmware clock", unsafe {
            (services.allocate_pool)(
                efi::LOADER_DATA,
                core::mem::size_of::<TimerCounter>(),
                &mut storage,
            )
        })?;
        let counter =
            NonNull::new(storage.cast::<TimerCounter>()).ok_or(UefiError::InvalidFirmwareTable)?;
        unsafe {
            counter.as_ptr().write(TimerCounter {
                ticks: AtomicU64::new(0),
            })
        };
        let mut event = ptr::null_mut();
        let status = unsafe {
            (services.create_event)(
                efi::EVT_TIMER | efi::EVT_NOTIFY_SIGNAL,
                efi::TPL_NOTIFY,
                Some(timer_tick),
                storage,
                &mut event,
            )
        };
        if let Err(error) = check("create firmware clock", status) {
            unsafe { (services.free_pool)(storage) };
            return Err(error);
        }
        let clock = Self {
            context,
            source: ClockSource::Timer { event, counter },
            last_nanos: Cell::new(0),
        };
        // If arming fails, Drop closes the event before releasing callback data.
        check("arm firmware clock", unsafe {
            (services.set_timer)(event, efi::TIMER_PERIODIC, TIMER_PERIOD_NANOS / 100)
        })?;
        Ok(clock)
    }

    /// Timestamp-backed time or a coarse 10ms notification counter. Firmware
    /// may delay/coalesce timer notifications, so fallback time can undercount
    /// blocking operations. Observed exit freezes this clock without further
    /// protocol calls. This domain must not be compared with OS clock domains.
    pub fn now(&self) -> Duration {
        if !self.context.boot_services_active() {
            return Duration::from_nanos(self.last_nanos.get());
        }
        let nanos = match &self.source {
            ClockSource::Timestamp {
                protocol,
                frequency,
                end_value,
                last_counter,
                elapsed_ticks,
            } => {
                let current = unsafe { (protocol.as_ref().get_timestamp)() };
                let Some(delta) = counter_delta(last_counter.get(), current, *end_value) else {
                    return Duration::from_nanos(self.last_nanos.get());
                };
                last_counter.set(current);
                let ticks = elapsed_ticks.get().saturating_add(delta);
                elapsed_ticks.set(ticks);
                ticks_to_nanos(ticks, *frequency)
            }
            ClockSource::Timer { counter, .. } => unsafe { counter.as_ref() }
                .ticks
                .load(Ordering::Relaxed)
                .saturating_mul(TIMER_PERIOD_NANOS),
        };
        let monotonic = self.last_nanos.get().max(nanos);
        self.last_nanos.set(monotonic);
        Duration::from_nanos(monotonic)
    }

    pub fn is_high_resolution(&self) -> bool {
        matches!(self.source, ClockSource::Timestamp { .. })
    }
}

impl Drop for UefiClock<'_> {
    fn drop(&mut self) {
        if let ClockSource::Timer { event, counter } = &self.source {
            if let Ok(services) = self.context.services() {
                let status = unsafe { (services.close_event)(*event) };
                // Failed closure retains the timer's callback storage. After
                // exit, all loader memory is reclaimed by the receiving OS.
                if !status.is_error() {
                    unsafe { (services.free_pool)(counter.as_ptr().cast()) };
                }
            }
        }
    }
}

fn counter_delta(previous: u64, current: u64, end_value: u64) -> Option<u128> {
    if previous > end_value || current > end_value {
        return None;
    }
    Some(if current >= previous {
        u128::from(current - previous)
    } else {
        u128::from(end_value - previous) + 1 + u128::from(current)
    })
}

fn ticks_to_nanos(ticks: u128, frequency: u64) -> u64 {
    let frequency = u128::from(frequency);
    let whole = (ticks / frequency).saturating_mul(1_000_000_000);
    let partial = (ticks % frequency) * 1_000_000_000 / frequency;
    u64::try_from(whole.saturating_add(partial)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_rollover_and_out_of_range_counter() {
        assert_eq!(counter_delta(250, 3, 255), Some(9));
        assert_eq!(counter_delta(u64::MAX - 2, 1, u64::MAX), Some(4));
        assert_eq!(counter_delta(10, 10, 255), Some(0));
        assert_eq!(counter_delta(10, 256, 255), None);
    }

    #[test]
    fn rational_frequency_conversion_does_not_overflow() {
        assert_eq!(ticks_to_nanos(3, 2), 1_500_000_000);
        assert_eq!(ticks_to_nanos(1, 3), 333_333_333);
        assert_eq!(ticks_to_nanos(u128::MAX, u64::MAX), u64::MAX);
    }
}

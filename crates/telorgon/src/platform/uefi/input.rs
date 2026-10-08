use core::ptr::{self, NonNull};
use core::time::Duration;
use r_efi::{efi, protocols::simple_text_input};

use super::{UefiContext, UefiError, UefiResult, check};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UefiKey {
    pub scan_code: u16,
    pub unicode: u16,
}

impl UefiKey {
    pub fn is_up(self) -> bool {
        self.scan_code == 1
    }
    pub fn is_down(self) -> bool {
        self.scan_code == 2
    }
    pub fn is_enter(self) -> bool {
        self.unicode == 13
    }
    pub fn is_escape(self) -> bool {
        self.scan_code == 23 || self.unicode == 27
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    Input,
    Timeout,
}

struct TimerEvent<'a> {
    context: &'a UefiContext,
    event: efi::Event,
}

impl Drop for TimerEvent<'_> {
    fn drop(&mut self) {
        if let Ok(services) = self.context.services() {
            unsafe { (services.close_event)(self.event) };
        }
    }
}

impl UefiContext {
    fn input_protocol(&self) -> UefiResult<NonNull<simple_text_input::Protocol>> {
        NonNull::new(self.table()?.con_in).ok_or(UefiError::InvalidFirmwareTable)
    }

    pub fn read_key(&self) -> UefiResult<Option<UefiKey>> {
        let input = self.input_protocol()?;
        let mut key = simple_text_input::InputKey::default();
        let status = unsafe { (input.as_ref().read_key_stroke)(input.as_ptr(), &mut key) };
        if status == efi::Status::NOT_READY {
            return Ok(None);
        }
        check("read keyboard", status)?;
        Ok(Some(UefiKey {
            scan_code: key.scan_code,
            unicode: key.unicode_char,
        }))
    }

    pub fn wait_for_input(&self, timeout: Option<Duration>) -> UefiResult<WaitOutcome> {
        let input = self.input_protocol()?;
        let services = self.services()?;
        let mut events = [unsafe { input.as_ref().wait_for_key }, ptr::null_mut()];
        if events[0].is_null() {
            return Err(UefiError::InvalidFirmwareTable);
        }
        let timer = if let Some(duration) = timeout {
            if duration.is_zero() {
                let status = unsafe { (services.check_event)(events[0]) };
                if status == efi::Status::NOT_READY {
                    return Ok(WaitOutcome::Timeout);
                }
                check("poll keyboard event", status)?;
                return Ok(WaitOutcome::Input);
            }
            let ticks = duration
                .as_nanos()
                .checked_add(99)
                .ok_or(UefiError::ResourceLimit)?
                / 100;
            let ticks = u64::try_from(ticks).map_err(|_| UefiError::ResourceLimit)?;
            check("create timer", unsafe {
                (services.create_event)(
                    efi::EVT_TIMER,
                    efi::TPL_APPLICATION,
                    None,
                    ptr::null_mut(),
                    &mut events[1],
                )
            })?;
            let event = TimerEvent {
                context: self,
                event: events[1],
            };
            check("arm timer", unsafe {
                (services.set_timer)(event.event, efi::TIMER_RELATIVE, ticks)
            })?;
            Some(event)
        } else {
            None
        };
        let mut selected = 0;
        check("wait for input", unsafe {
            (services.wait_for_event)(
                if timer.is_some() { 2 } else { 1 },
                events.as_mut_ptr(),
                &mut selected,
            )
        })?;
        Ok(if selected == 0 {
            WaitOutcome::Input
        } else {
            WaitOutcome::Timeout
        })
    }
}

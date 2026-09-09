//! Asynchronous X11 selection conversion. The host must use a private requestor,
//! reserve a distinct property for each live attempt, and cancel on ownership loss.
use super::{
    Error, Result,
    incr_wire::PropertyTarget,
    requests::{Completion, Importance, ReplyKind, RequestId, Requests},
    transport::Transport,
};
use std::time::Instant;
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Available,
    Refused,
    Failed,
    Cancelled,
}

pub struct Conversion {
    id: RequestId,
    destination: PropertyTarget,
    selection: u32,
    target: u32,
    time: u32,
    deadline: Instant,
    state: State,
}
impl Conversion {
    /// Use an actual nonzero server timestamp. Do not reuse the requestor/property
    /// identity for a new attempt while old notifications could still arrive.
    pub fn begin(
        destination: PropertyTarget,
        selection: u32,
        target: u32,
        time: u32,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<Self> {
        destination.validate(requests)?;
        if selection == 0 || target == 0 || time == 0 {
            return Err(Error("invalid selection conversion context".into()));
        }
        let (bytes, _) = Request::serialize(
            xproto::ConvertSelectionRequest {
                requestor: destination.window,
                selection,
                target,
                property: destination.property,
                time,
            },
            0,
        );
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Void,
            Importance::Optional,
            deadline,
        )?;
        Ok(Self {
            id,
            destination,
            selection,
            target,
            time,
            deadline,
            state: State::Waiting,
        })
    }
    pub fn state(&self) -> State {
        self.state
    }
    /// The property may contain direct data or an INCR header. Read and validate
    /// it through PropertyRead before deciding which transfer path to use.
    pub fn property(&self) -> Option<PropertyTarget> {
        (self.state == State::Available).then_some(self.destination)
    }
    pub fn deadline(&self) -> Option<Instant> {
        (self.state == State::Waiting).then_some(self.deadline)
    }
    pub fn expire(&mut self, now: Instant) {
        if self.state == State::Waiting && now >= self.deadline {
            self.state = State::Failed;
        }
    }
    pub fn cancel(&mut self) {
        if matches!(self.state, State::Waiting | State::Available) {
            self.state = State::Cancelled;
        }
    }
    /// Route completions from the same authenticated connection. A checked void
    /// request is not a conversion result; a silent owner still hits our deadline.
    pub fn completion(&mut self, completion: &Completion) -> bool {
        if self.state != State::Waiting {
            return false;
        }
        match completion {
            Completion::Error(id, _) | Completion::TimedOut(id) if *id == self.id => {
                self.state = State::Failed;
                true
            }
            Completion::Checked(id) if *id == self.id => true,
            _ => false,
        }
    }
    pub fn event(&mut self, generation: u64, bytes: &[u8], now: Instant) -> Result<bool> {
        self.expire(now);
        if generation != self.id.generation || self.state != State::Waiting {
            return Ok(false);
        }
        // Selection owners send synthetic SelectionNotify via SendEvent.
        if bytes.first().map(|b| b & 0x7f) != Some(xproto::SELECTION_NOTIFY_EVENT) {
            return Ok(false);
        }
        if bytes.len() != 32 {
            self.state = State::Failed;
            return Err(Error("malformed selection conversion notification".into()));
        }
        let (event, _) = xproto::SelectionNotifyEvent::try_parse(bytes)
            .map_err(|_| Error("malformed selection conversion notification".into()))?;
        if event.requestor != self.destination.window
            || event.selection != self.selection
            || event.target != self.target
            || event.time != self.time
            || (event.property != 0 && event.property != self.destination.property)
        {
            return Ok(false);
        }
        self.state = if event.property == 0 {
            State::Refused
        } else {
            State::Available
        };
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xwayland::discovery::tests::{flush_requests, ready};
    use std::{io::Read, time::Duration};
    #[test]
    fn conversion_matches_notify_and_retains_local_timeout_after_checked_request() {
        for outcome in [State::Available, State::Refused, State::Failed] {
            let (mut transport, mut requests, _, mut peer, now) = ready();
            let deadline = now + Duration::from_secs(10);
            let destination = PropertyTarget {
                generation: 1,
                window: 10,
                property: 20,
            };
            let mut conversion = Conversion::begin(
                destination,
                30,
                40,
                50,
                &mut transport,
                &mut requests,
                deadline,
            )
            .unwrap();
            flush_requests(&mut transport);
            let mut wire = [0u8; 24];
            peer.read_exact(&mut wire).unwrap();
            assert_eq!(wire[0], 24); // ConvertSelection
            for (index, value) in [10u32, 30, 40, 20, 50].into_iter().enumerate() {
                assert_eq!(
                    u32::from_ne_bytes(wire[4 + index * 4..8 + index * 4].try_into().unwrap()),
                    value
                );
            }
            assert!(conversion.completion(&Completion::Checked(conversion.id)));
            assert_eq!(conversion.deadline(), Some(deadline));
            let mut event = xproto::SelectionNotifyEvent {
                response_type: xproto::SELECTION_NOTIFY_EVENT | 0x80,
                sequence: 1,
                requestor: 10,
                selection: 30,
                target: 40,
                time: 50,
                property: if outcome == State::Refused { 0 } else { 20 },
            };
            let wire: [u8; 32] = event.into();
            assert!(!conversion.event(6, &wire, now).unwrap());
            event.target = 41;
            assert!(!conversion.event(1, &<[u8; 32]>::from(event), now).unwrap());
            if outcome == State::Failed {
                conversion.expire(deadline);
            }
            assert_eq!(
                conversion.event(1, &wire, now).unwrap(),
                outcome != State::Failed
            );
            assert_eq!(conversion.state(), outcome);
            assert_eq!(conversion.property().is_some(), outcome == State::Available);
            assert!(!conversion.event(1, &wire, now).unwrap());
            conversion.cancel();
            assert!(conversion.property().is_none());
        }
    }
}

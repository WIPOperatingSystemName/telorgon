//! Asynchronous policy commands. Reply barriers confirm void requests; command
//! failure is reported once and never mutates confirmed window geometry/map state.
use super::{
    Error, Result,
    association::XWindow,
    requests::{Completion, Importance, ReplyKind, Requests},
    transport::Transport,
    window::{Action, Windows},
};
use std::{collections::BTreeMap, time::Instant};
use x11rb_protocol::{
    protocol::xproto,
    x11_utils::{Request, TryParse},
};
struct Group {
    window: XWindow,
    remaining: u8,
    failed: bool,
}
pub(crate) struct Commands {
    pending: BTreeMap<u64, (u64, bool)>,
    groups: BTreeMap<u64, Group>,
}
impl Commands {
    pub fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }
    pub fn pending_for(&self, window: XWindow) -> bool {
        self.groups.values().any(|group| group.window == window)
    }
    pub fn available_groups(&self) -> usize {
        256 - self.groups.len()
    }
    pub fn queue(
        &mut self,
        request: impl Request,
        window: XWindow,
        transport: &mut Transport,
        requests: &mut Requests,
        deadline: Instant,
    ) -> Result<()> {
        if self.groups.len() >= 256 {
            return Err(Error("XWM command bound reached".into()));
        }
        let (bytes, fds) = Request::serialize(request, 0);
        if !fds.is_empty() {
            return Err(Error("unexpected XWM command descriptors".into()));
        }
        let id = requests.queue(
            transport,
            bytes,
            ReplyKind::Void,
            Importance::Optional,
            deadline,
        )?;
        self.groups.insert(
            id.sequence,
            Group {
                window,
                remaining: 1,
                failed: false,
            },
        );
        self.pending.insert(id.sequence, (id.sequence, false));
        let (bytes, _) = Request::serialize(xproto::GetInputFocusRequest, 0);
        let barrier = requests.queue(
            transport,
            bytes,
            ReplyKind::Reply,
            Importance::Optional,
            deadline,
        )?;
        self.groups.get_mut(&id.sequence).unwrap().remaining += 1;
        self.pending.insert(barrier.sequence, (id.sequence, true));
        Ok(())
    }
    pub fn completion(
        &mut self,
        completion: &Completion,
        windows: &Windows,
        actions: &mut Vec<Action>,
    ) -> Result<bool> {
        let id = match completion {
            Completion::Reply(id, _)
            | Completion::Checked(id)
            | Completion::Error(id, _)
            | Completion::TimedOut(id) => id,
            _ => return Ok(false),
        };
        let Some(&(group_id, barrier)) = self.pending.get(&id.sequence) else {
            return Ok(false);
        };
        let group = self.groups.get_mut(&group_id).unwrap();
        if group.window.generation != id.generation {
            return Ok(false);
        }
        if let Completion::Reply(_, bytes) = completion {
            if !barrier {
                return Err(Error("unexpected XWM command reply".into()));
            }
            xproto::GetInputFocusReply::try_parse(bytes)
                .map_err(|_| Error("malformed XWM command barrier".into()))?;
        }
        if matches!(
            completion,
            Completion::Error(_, _) | Completion::TimedOut(_)
        ) && !group.failed
        {
            group.failed = true;
            if windows
                .get(group.window.xid)
                .is_some_and(|w| w.id == group.window)
            {
                actions.push(Action::CommandFailed(group.window));
            }
        }
        self.pending.remove(&id.sequence);
        group.remaining -= 1;
        if group.remaining == 0 {
            self.groups.remove(&group_id);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::tests::ready;
    use super::super::requests::RequestId;
    use super::*;
    use std::time::Duration;
    fn create(windows: &mut Windows) -> XWindow {
        let bytes: [u8; 32] = xproto::CreateNotifyEvent {
            response_type: xproto::CREATE_NOTIFY_EVENT,
            parent: 1,
            window: 10,
            width: 10,
            height: 10,
            ..Default::default()
        }
        .into();
        windows.event(&bytes).unwrap();
        windows.get(10).unwrap().id
    }
    #[test]
    fn command_and_barrier_timeouts_emit_only_one_failure() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        let mut windows = Windows::new(1, 16, 1, 2, 100).unwrap();
        let window = create(&mut windows);
        let mut commands = Commands::new();
        commands
            .queue(
                xproto::MapWindowRequest { window: 10 },
                window,
                &mut transport,
                &mut requests,
                now + Duration::from_secs(1),
            )
            .unwrap();
        let mut actions = vec![];
        for completion in requests.expire(now + Duration::from_secs(2)).unwrap() {
            assert!(
                commands
                    .completion(&completion, &windows, &mut actions)
                    .unwrap()
            );
        }
        assert_eq!(actions, vec![Action::CommandFailed(window)]);
        assert!(commands.groups.is_empty());
        assert!(commands.pending.is_empty());
        assert!(!windows.get(10).unwrap().mapped);
    }
    #[test]
    fn stale_command_error_cannot_target_reused_xid() {
        let (mut transport, mut requests, _, _peer, now) = ready();
        let mut windows = Windows::new(1, 16, 1, 2, 100).unwrap();
        let old = create(&mut windows);
        let mut commands = Commands::new();
        commands
            .queue(
                xproto::MapWindowRequest { window: 10 },
                old,
                &mut transport,
                &mut requests,
                now,
            )
            .unwrap();
        let bytes: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        windows.event(&bytes).unwrap();
        let new = create(&mut windows);
        assert_ne!(old, new);
        let mut actions = vec![];
        assert!(
            commands
                .completion(
                    &Completion::Error(
                        RequestId {
                            generation: 1,
                            sequence: 33
                        },
                        3
                    ),
                    &windows,
                    &mut actions
                )
                .unwrap()
        );
        assert!(actions.is_empty());
        assert_eq!(windows.get(10).unwrap().id, new);
    }
}

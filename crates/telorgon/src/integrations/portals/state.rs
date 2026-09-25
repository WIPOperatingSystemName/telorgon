//! Bounded backend state. D-Bus identity is checked before entering this state machine.

use super::{Lease, StartCompletion, StartRequest};
use crate::shell::capture::CaptureOptions;
use std::collections::BTreeMap;
use std::sync::Arc;
use zbus::zvariant::OwnedValue;

pub(super) type Options = std::collections::HashMap<String, OwnedValue>;
pub(super) const MAX_SESSIONS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    options: CaptureOptions,
    source_types: u32,
    multiple: bool,
    persistence: u32,
    restore: Option<super::restore::RestoreData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Created,
    Selected(Selection),
    Starting,
    Streaming,
}

pub(super) struct Session {
    pub owner: String,
    pub app_id: String,
    pub requester: u64,
    pub lease: Arc<Lease>,
    pub phase: Phase,
    pub request_path: Option<String>,
    pub registering: bool,
}

pub(super) struct Sessions {
    next_id: u64,
    pub available_source_types: u32,
    pub failures: std::collections::VecDeque<String>,
    pub grant_revision: u64,
    pub entries: BTreeMap<String, Session>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            next_id: 1,
            available_source_types: super::AVAILABLE_SOURCE_TYPES,
            failures: Default::default(),
            grant_revision: 0,
            entries: BTreeMap::new(),
        }
    }
}

impl Sessions {
    pub fn with_source_types(available_source_types: u32) -> Self {
        Self {
            available_source_types,
            ..Self::default()
        }
    }

    pub fn create(
        &mut self,
        path: &str,
        owner: &str,
        app_id: &str,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Lease>, ()> {
        if !valid_path(path, "session")
            || app_id.len() > 255
            || self.entries.contains_key(path)
            || self.entries.len() >= MAX_SESSIONS
            || self
                .entries
                .values()
                .filter(|s| s.owner == owner && s.app_id == app_id)
                .count()
                >= 2
        {
            return Err(());
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(())?;
        let requester = self
            .entries
            .values()
            .find(|s| s.owner == owner && s.app_id == app_id)
            .map_or(id, |s| s.requester);
        let lease = Arc::new(Lease::new(id, wake));
        self.entries.insert(
            path.into(),
            Session {
                owner: owner.into(),
                app_id: app_id.into(),
                requester,
                lease: lease.clone(),
                phase: Phase::Created,
                request_path: None,
                registering: true,
            },
        );
        Ok(lease)
    }

    pub fn registered(&mut self, path: &str, id: u64) -> bool {
        let Some(session) = self.entries.get_mut(path).filter(|s| s.lease.id == id) else {
            return false;
        };
        session.registering = false;
        true
    }

    pub fn owned(&mut self, path: &str, owner: &str, app_id: &str) -> Result<&mut Session, ()> {
        self.entries
            .get_mut(path)
            .filter(|s| {
                s.owner == owner && s.app_id == app_id && !s.registering && !s.lease.closed()
            })
            .ok_or(())
    }

    pub fn select(
        &mut self,
        path: &str,
        owner: &str,
        app_id: &str,
        options: &Options,
    ) -> Result<(), ()> {
        let available = self.available_source_types;
        let session = self.owned(path, owner, app_id)?;
        let validated = selection(options).and_then(|mut selection| {
            selection.source_types &= available;
            if selection.source_types == 0 {
                Err(())
            } else {
                selection.restore = options.get("restore_data").and_then(|value| {
                    super::restore::RestoreData::decode(
                        value,
                        app_id,
                        selection.source_types,
                        selection.multiple,
                    )
                });
                Ok(selection)
            }
        });
        if session.phase != Phase::Created || validated.is_err() {
            session.lease.close();
            return Err(());
        }
        session.phase = Phase::Selected(validated?);
        Ok(())
    }

    pub fn begin(
        &mut self,
        path: &str,
        owner: &str,
        app_id: &str,
        request_path: &str,
    ) -> Result<
        (
            StartRequest,
            async_channel::Receiver<Result<StartCompletion, u32>>,
        ),
        (),
    > {
        if !valid_path(request_path, "request")
            || self
                .entries
                .values()
                .any(|s| s.request_path.as_deref() == Some(request_path))
        {
            return Err(());
        }
        let session = self.owned(path, owner, app_id)?;
        let Phase::Selected(selection) = session.phase.clone() else {
            return Err(());
        };
        session.phase = Phase::Starting;
        session.request_path = Some(request_path.into());
        let (reply, receive) = async_channel::bounded(1);
        Ok((
            StartRequest {
                requester: session.requester,
                app_id: session.app_id.clone(),
                lease: session.lease.clone(),
                options: selection.options,
                source_types: selection.source_types,
                multiple: selection.multiple,
                persistence: selection.persistence,
                restore: selection.restore,
                reply,
            },
            receive,
        ))
    }

    pub fn finish(&mut self, path: &str, id: u64, success: bool) -> bool {
        let Some(session) = self.entries.get_mut(path).filter(|s| s.lease.id == id) else {
            return false;
        };
        session.request_path = None;
        if !success || session.lease.closed() || session.phase != Phase::Starting {
            session.lease.close();
            return false;
        }
        session.phase = Phase::Streaming;
        true
    }

    pub fn attach_restore(
        &mut self,
        lease: &Arc<Lease>,
        grant: super::restore::RestoreData,
        revision: u64,
    ) -> bool {
        if revision == u64::MAX
            || self.grant_revision != revision
            || lease.closed()
            || !self
                .entries
                .values()
                .any(|s| Arc::ptr_eq(&s.lease, lease) && s.app_id == grant.app_id)
        {
            return false;
        }
        *lease.saved.lock().unwrap_or_else(|e| e.into_inner()) = Some(grant);
        true
    }
    pub fn invalidate_application(&mut self, app: &str) {
        self.grant_revision = self.grant_revision.saturating_add(1);
        for session in self.entries.values().filter(|s| s.app_id == app) {
            session.lease.close();
        }
    }
    pub fn invalidate_grant(&mut self, app: &str, grant_id: &str) {
        self.grant_revision = self.grant_revision.saturating_add(1);
        for session in self.entries.values() {
            if session
                .lease
                .saved
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|g| g.app_id == app && g.grant_id == grant_id)
            {
                session.lease.close();
            }
        }
    }

    pub fn revoke_except(&self, owner: Option<&str>) {
        for session in self.entries.values() {
            if Some(session.owner.as_str()) != owner {
                session.lease.close();
            }
        }
    }
}

pub(super) fn valid_path(path: &str, kind: &str) -> bool {
    path.len() <= 512
        && path.starts_with(&format!("/org/freedesktop/portal/desktop/{kind}/"))
        && zbus::zvariant::ObjectPath::try_from(path).is_ok()
}

fn uint(options: &Options, key: &str, default: u32) -> Result<u32, ()> {
    options
        .get(key)
        .map_or(Ok(default), |v| u32::try_from(v).map_err(|_| ()))
}

fn selection(options: &Options) -> Result<Selection, ()> {
    let types = uint(options, "types", 1)?;
    let cursor = uint(options, "cursor_mode", 1)?;
    if types == 0
        || types & !7 != 0
        || types & super::AVAILABLE_SOURCE_TYPES == 0
        || !matches!(cursor, 1 | 2 | 4)
    {
        return Err(());
    }
    let multiple = options
        .get("multiple")
        .map_or(Ok(false), |value| bool::try_from(value).map_err(|_| ()))?;
    let persistence = uint(options, "persist_mode", 0)?;
    if persistence > 2 {
        return Err(());
    }
    Ok(Selection {
        source_types: types,
        multiple,
        persistence,
        restore: None,
        options: CaptureOptions::new(
            match cursor {
                2 => crate::shell::capture::CaptureCursorMode::Embedded,
                4 => crate::shell::capture::CaptureCursorMode::Metadata,
                _ => crate::shell::capture::CaptureCursorMode::Hidden,
            },
            CaptureOptions::default().max_frame_rate(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(i: u32) -> String {
        format!("/org/freedesktop/portal/desktop/session/test/{i}")
    }
    fn request(i: u32) -> String {
        format!("/org/freedesktop/portal/desktop/request/test/{i}")
    }
    fn create(s: &mut Sessions, i: u32, app: &str) -> Arc<Lease> {
        let lease = s.create(&path(i), ":1.2", app, Arc::new(|| {})).unwrap();
        assert!(s.registered(&path(i), lease.id));
        lease
    }

    #[test]
    fn forgetting_an_application_closes_its_pending_and_active_leases_only() {
        let mut state = Sessions::default();
        let a = create(&mut state, 1, "app.a");
        let b = create(&mut state, 2, "app.a");
        let other = create(&mut state, 3, "app.b");
        state.invalidate_application("app.a");
        assert!(a.closed());
        assert!(b.closed());
        assert!(!other.closed());
        assert_eq!(state.grant_revision, 1);
    }

    #[test]
    fn revocation_closes_all_matching_leases_and_rejects_racing_lookup_results() {
        let mut sessions = Sessions::default();
        let a = create(&mut sessions, 1, "app");
        let b = create(&mut sessions, 2, "app");
        let other = create(&mut sessions, 3, "other");
        let pending = sessions
            .create(&path(4), ":1.3", "app", Arc::new(|| {}))
            .unwrap();
        assert!(sessions.registered(&path(4), pending.id));
        let grant = super::super::restore::RestoreData {
            app_id: "app".into(),
            grant_id: "0".repeat(64),
            sources: vec![super::super::restore::RestoreSource {
                kind: 1,
                identity: "output".into(),
            }],
        };
        let revision = sessions.grant_revision;
        assert!(sessions.attach_restore(&a, grant.clone(), revision));
        assert!(sessions.attach_restore(&b, grant.clone(), revision));
        sessions.invalidate_grant("app", &grant.grant_id);
        assert!(a.closed());
        assert!(b.closed());
        assert!(!other.closed());
        assert!(!sessions.attach_restore(&pending, grant, revision));
        assert!(!pending.has_saved_grant());
        assert!(!pending.closed());
    }

    #[test]
    fn restore_hints_are_app_bound_and_malformed_hints_fall_back_to_consent() {
        use super::super::restore::{RestoreData, RestoreSource};
        let record = RestoreData {
            app_id: "app".into(),
            grant_id: "01".repeat(32),
            sources: vec![RestoreSource {
                kind: 1,
                identity: "host-output-key".into(),
            }],
        };
        for case in 0..4 {
            let mut sessions = Sessions::default();
            let app = if case == 1 { "other" } else { "app" };
            create(&mut sessions, 1, app);
            let mut options = Options::from([
                ("persist_mode".into(), 2u32.into()),
                (
                    "restore_data".into(),
                    if case == 2 {
                        42u32.into()
                    } else {
                        record.encode().unwrap()
                    },
                ),
            ]);
            if case == 3 {
                options.insert("types".into(), 2u32.into());
            }
            sessions.select(&path(1), ":1.2", app, &options).unwrap();
            let (start, _) = sessions.begin(&path(1), ":1.2", app, &request(1)).unwrap();
            assert_eq!(start.persistence, 2);
            assert_eq!(start.restore.is_some(), case == 0);
            // A well-formed hint still starts an ordinary host consent request.
            assert_eq!(sessions.entries[&path(1)].phase, Phase::Starting);
            assert!(!start.lease.closed());
        }
    }

    #[test]
    fn multiple_selection_survives_until_host_start() {
        for multiple in [false, true] {
            let mut sessions = Sessions::default();
            create(&mut sessions, 1, "app");
            sessions
                .select(
                    &path(1),
                    ":1.2",
                    "app",
                    &Options::from([("multiple".into(), multiple.into())]),
                )
                .unwrap();
            let (start, _) = sessions
                .begin(&path(1), ":1.2", "app", &request(1))
                .unwrap();
            assert_eq!(start.multiple, multiple);
        }
        assert!(!selection(&Options::new()).unwrap().multiple);
    }

    #[test]
    fn capture_config_filters_mixed_source_requests_and_rejects_disabled_sources() {
        for available in [1u32, 2] {
            let mut sessions = Sessions::with_source_types(available);
            create(&mut sessions, 1, "app");
            sessions
                .select(
                    &path(1),
                    ":1.2",
                    "app",
                    &Options::from([("types".into(), 3u32.into())]),
                )
                .unwrap();
            let (request, _) = sessions
                .begin(&path(1), ":1.2", "app", &request(1))
                .unwrap();
            assert_eq!(request.source_types, available);
            let lease = create(&mut sessions, 2, "other-app");
            assert!(
                sessions
                    .select(
                        &path(2),
                        ":1.2",
                        "other-app",
                        &Options::from([("types".into(), (3 ^ available).into())])
                    )
                    .is_err()
            );
            assert!(lease.closed());
        }
    }

    #[test]
    fn registration_must_finish_before_selection_and_cancellation_is_not_undone() {
        let mut s = Sessions::default();
        let lease = s.create(&path(1), ":1.2", "app", Arc::new(|| {})).unwrap();
        assert!(s.select(&path(1), ":1.2", "app", &Options::new()).is_err());
        lease.close();
        assert!(s.registered(&path(1), lease.id));
        assert!(s.select(&path(1), ":1.2", "app", &Options::new()).is_err());
        s.entries.remove(&path(1));
        let replacement = s.create(&path(1), ":1.2", "app", Arc::new(|| {})).unwrap();
        assert!(!s.registered(&path(1), lease.id));
        assert!(s.entries[&path(1)].registering);
        assert!(!replacement.closed());
    }

    #[test]
    fn identity_and_order_cannot_be_bypassed() {
        let mut s = Sessions::default();
        let lease = create(&mut s, 1, "app");
        assert!(s.begin(&path(1), ":1.2", "app", &request(1)).is_err());
        assert!(s.select(&path(1), ":1.3", "app", &Options::new()).is_err());
        assert!(
            s.select(&path(1), ":1.2", "other", &Options::new())
                .is_err()
        );
        assert!(!lease.closed());
        s.select(&path(1), ":1.2", "app", &Options::new()).unwrap();
        let (_, reply) = s.begin(&path(1), ":1.2", "app", &request(1)).unwrap();
        assert!(s.begin(&path(1), ":1.2", "app", &request(2)).is_err());
        drop(reply);
        assert!(s.finish(&path(1), lease.id, true));
        assert_eq!(s.entries[&path(1)].phase, Phase::Streaming);
    }

    #[test]
    fn cancellation_dominates_late_stream_readiness_and_owner_replacement() {
        let mut s = Sessions::default();
        let lease = create(&mut s, 1, "app");
        s.select(&path(1), ":1.2", "app", &Options::new()).unwrap();
        let (request, _) = s.begin(&path(1), ":1.2", "app", &request(1)).unwrap();
        s.revoke_except(Some(":1.3"));
        assert!(request.lease.closed());
        assert!(!s.finish(&path(1), lease.id, true));
        lease.close();
        assert!(futures_lite::future::block_on(lease.cancelled.recv()).is_err());
    }

    #[test]
    fn bounds_and_nonrecycling_identities_survive_retirement() {
        let mut s = Sessions::default();
        let first = create(&mut s, 1, "app");
        create(&mut s, 2, "app");
        assert!(s.create(&path(3), ":1.2", "app", Arc::new(|| {})).is_err());
        assert!(
            s.create(&path(1), ":1.2", "other", Arc::new(|| {}))
                .is_err()
        );
        for i in 3..=8 {
            create(&mut s, i, &format!("app{i}"));
        }
        assert!(s.create(&path(9), ":1.2", "more", Arc::new(|| {})).is_err());
        s.entries.remove(&path(1));
        let next = create(&mut s, 1, "app");
        assert!(next.id > first.id);
        assert!(!s.finish(&path(1), first.id, true));
        assert!(!next.closed());
    }

    #[test]
    fn unsupported_and_mistyped_capabilities_are_rejected() {
        for options in [
            Options::from([("types".into(), 4u32.into())]),
            Options::from([("types".into(), 0u32.into())]),
            Options::from([("cursor_mode".into(), 8u32.into())]),
            Options::from([("cursor_mode".into(), 3u32.into())]),
            Options::from([("multiple".into(), 1u32.into())]),
            Options::from([("persist_mode".into(), 3u32.into())]),
        ] {
            assert!(selection(&options).is_err());
        }
        assert!(
            selection(&Options::from([
                ("types".into(), 3u32.into()),
                ("multiple".into(), true.into())
            ]))
            .is_ok()
        );
    }

    #[test]
    fn separate_cursor_choice_is_preserved_for_host_admission() {
        let selected = selection(&Options::from([("cursor_mode".into(), 4u32.into())])).unwrap();
        assert_eq!(
            selected.options.cursor(),
            crate::shell::capture::CaptureCursorMode::Metadata
        );
    }

    #[test]
    fn embedded_cursor_choice_is_preserved_for_host_admission() {
        let options = selection(&Options::from([("cursor_mode".into(), 2u32.into())])).unwrap();
        assert_eq!(
            options.options.cursor(),
            crate::shell::capture::CaptureCursorMode::Embedded
        );
        assert_eq!(
            selection(&Options::new()).unwrap().options.cursor(),
            crate::shell::capture::CaptureCursorMode::Hidden
        );
    }

    #[test]
    fn requested_source_types_survive_selection_and_start() {
        use crate::shell::{OutputId, WindowId, capture::CaptureSource};
        use std::num::NonZeroU32;
        let window = CaptureSource::Window(WindowId::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(2).unwrap(),
        ));
        for types in [1u32, 2, 3] {
            let mut sessions = Sessions::default();
            create(&mut sessions, 1, "app");
            sessions
                .select(
                    &path(1),
                    ":1.2",
                    "app",
                    &Options::from([("types".into(), types.into())]),
                )
                .unwrap();
            let (request, _) = sessions
                .begin(&path(1), ":1.2", "app", &request(1))
                .unwrap();
            assert_eq!(request.source_types, types);
            assert_eq!(
                request.allows_source(CaptureSource::Output(OutputId::MIN)),
                types & 1 != 0
            );
            assert_eq!(request.allows_source(window), types & 2 != 0);
        }
    }
}

//! Deliberate advanced graph editing. Links are leases: dropping the lease removes the
//! client-owned link; existing external links are never destroyed. Feedback is rejected by
//! default and requires explicit opt-in (including a correctly designed delay elsewhere).
use super::{
    connection::{Shared, native},
    *,
};
use pipewire::proxy::ProxyT;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback {
    Reject,
    Allow,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    Paused,
    Active,
    Removed,
    Failed(MediaError),
}
pub struct LinkLease {
    pub(crate) state: Arc<Mutex<LinkState>>,
    pub(crate) stop: Arc<AtomicBool>,
    connection: ConnectionHandle,
}
impl LinkLease {
    pub fn state(&self) -> LinkState {
        if !matches!(self.connection.state(), ConnectionState::Ready) {
            return LinkState::Failed(MediaError::Disconnected);
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn remove(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for LinkLease {
    fn drop(&mut self) {
        self.remove();
    }
}
#[derive(Clone)]
pub struct Graph {
    connection: ConnectionHandle,
}
impl Graph {
    pub fn new(connection: ConnectionHandle) -> Self {
        Self { connection }
    }
    /// Select handles from the current registry. Acceptance is asynchronous; inspect lease
    /// state or registry link events. At most 128 owned links per connection.
    pub fn link(
        &self,
        output: ObjectHandle,
        input: ObjectHandle,
        feedback: Feedback,
    ) -> Result<LinkLease, MediaError> {
        self.connection.ensure_ready()?;
        validate(
            &self.connection.snapshot(),
            output,
            input,
            feedback,
            std::iter::empty(),
        )?;
        let lease = LinkLease {
            state: Arc::new(Mutex::new(LinkState::Connecting)),
            stop: Arc::new(AtomicBool::new(false)),
            connection: self.connection.clone(),
        };
        self.connection
            .send(super::connection::Command::Link(LinkCreation {
                output,
                input,
                feedback,
                state: lease.state.clone(),
                stop: lease.stop.clone(),
            }))?;
        Ok(lease)
    }
}
pub(crate) struct LinkCreation {
    output: ObjectHandle,
    input: ObjectHandle,
    feedback: Feedback,
    state: Arc<Mutex<LinkState>>,
    stop: Arc<AtomicBool>,
}
fn validate(
    snapshot: &RegistrySnapshot,
    output: ObjectHandle,
    input: ObjectHandle,
    feedback: Feedback,
    pending: impl Iterator<Item = (u32, u32)>,
) -> Result<(u32, u32), MediaError> {
    let a = snapshot.resolve(output)?;
    let b = snapshot.resolve(input)?;
    if a.kind != ObjectKind::Port
        || b.kind != ObjectKind::Port
        || a.properties.get("port.direction").map(String::as_str) != Some("out")
        || b.properties.get("port.direction").map(String::as_str) != Some("in")
    {
        return Err(MediaError::InvalidArgument("link port directions"));
    }
    let node = |o: &RegistryObject| {
        o.properties
            .get("node.id")
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or(MediaError::Unsupported("port node identity"))
    };
    let (from, to) = (node(a)?, node(b)?);
    if feedback == Feedback::Reject {
        let mut edges: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for link in snapshot.objects_of_kind(ObjectKind::Link) {
            if let (Some(a), Some(b)) = (
                link.properties
                    .get("link.output.node")
                    .and_then(|s| s.parse().ok()),
                link.properties
                    .get("link.input.node")
                    .and_then(|s| s.parse().ok()),
            ) {
                edges.entry(a).or_default().push(b);
            }
        }
        for (a, b) in pending {
            edges.entry(a).or_default().push(b);
        }
        let mut stack = vec![to];
        let mut seen = BTreeSet::new();
        while let Some(node) = stack.pop() {
            if node == from {
                return Err(MediaError::InvalidArgument(
                    "feedback loop requires explicit opt-in",
                ));
            }
            if seen.insert(node) {
                if let Some(next) = edges.get(&node) {
                    stack.extend(next);
                }
            }
        }
    }
    Ok((from, to))
}
pub(crate) struct NativeLink {
    listener: Option<pipewire::link::LinkListener>,
    proxy_listener: Option<pipewire::proxy::ProxyListener>,
    edge: (u32, u32),
    deadline: std::time::Instant,
    proxy: Option<pipewire::link::Link>,
    _core: pipewire::core::CoreRc,
    creation: LinkCreation,
}
impl NativeLink {
    pub fn create(
        core: pipewire::core::CoreRc,
        creation: LinkCreation,
        shared: &Shared,
        existing: &[NativeLink],
    ) -> Result<Self, MediaError> {
        let result: Result<_, MediaError> = (|| {
            if creation.stop.load(Ordering::Acquire) {
                return Err(MediaError::Cancelled);
            }
            let (from, to) = validate(
                &shared
                    .registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .snapshot,
                creation.output,
                creation.input,
                creation.feedback,
                existing
                    .iter()
                    .filter(|link| link.alive())
                    .map(|link| link.edge),
            )?;
            let proxy=core.create_object::<pipewire::link::Link>("link-factory",&pipewire::properties::properties! {
                "link.output.node"=>from.to_string(),"link.input.node"=>to.to_string(),"link.output.port"=>creation.output.id().to_string(),"link.input.port"=>creation.input.id().to_string(),
                "object.linger"=>"false","link.passive"=>"false",
            }).map_err(native)?;
            let state = creation.state.clone();
            let listener = proxy
                .add_listener_local()
                .info(move |info| {
                    *state.lock().unwrap_or_else(|e| e.into_inner()) = match info.state() {
                        pipewire::link::LinkState::Active => LinkState::Active,
                        pipewire::link::LinkState::Paused => LinkState::Paused,
                        pipewire::link::LinkState::Error(e) => LinkState::Failed(native(e)),
                        pipewire::link::LinkState::Unlinked => LinkState::Removed,
                        _ => LinkState::Connecting,
                    };
                })
                .register();
            let removed = creation.state.clone();
            let error = creation.state.clone();
            let proxy_listener = proxy
                .upcast_ref()
                .add_listener_local()
                .removed(move || {
                    let mut state = removed.lock().unwrap_or_else(|e| e.into_inner());
                    if !matches!(*state, LinkState::Failed(_)) {
                        *state = LinkState::Removed;
                    }
                })
                .error(move |_, _, message| {
                    *error.lock().unwrap_or_else(|e| e.into_inner()) =
                        LinkState::Failed(native(message));
                })
                .register();
            Ok((proxy, listener, proxy_listener, (from, to)))
        })();
        match result {
            Ok((proxy, listener, proxy_listener, edge)) => Ok(Self {
                listener: Some(listener),
                proxy_listener: Some(proxy_listener),
                edge,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
                proxy: Some(proxy),
                _core: core,
                creation,
            }),
            Err(error) => {
                *creation.state.lock().unwrap_or_else(|e| e.into_inner()) =
                    LinkState::Failed(error.clone());
                Err(error)
            }
        }
    }
    pub fn alive(&self) -> bool {
        {
            let mut state = self
                .creation
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if *state == LinkState::Connecting && std::time::Instant::now() >= self.deadline {
                *state = LinkState::Failed(MediaError::Timeout);
            }
        }
        !self.creation.stop.load(Ordering::Acquire)
            && !matches!(
                *self
                    .creation
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
                LinkState::Removed | LinkState::Failed(_)
            )
    }
}
impl Drop for NativeLink {
    fn drop(&mut self) {
        self.listener.take();
        self.proxy_listener.take();
        // object.linger=false ties server lifetime to this proxy. Do not send an
        // explicit core.destroy as well: the server may already have removed the link
        // after port teardown, and a second destroy targets a nonexistent resource.
        self.proxy.take();
        let mut state = self
            .creation
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, LinkState::Failed(_)) {
            *state = LinkState::Removed;
        }
    }
}
impl LinkCreation {
    pub(crate) fn fail(&self, error: MediaError) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = LinkState::Failed(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_checks_include_links_accepted_before_registry_publication() {
        let mut registry = super::super::registry::RegistryState::new(1, false, 16);
        let mut port = |id: u32, node: u32, direction: &str| {
            registry
                .insert(
                    id,
                    ObjectKind::Port,
                    7,
                    BTreeMap::from([
                        ("node.id".into(), node.to_string()),
                        ("port.direction".into(), direction.into()),
                    ]),
                )
                .unwrap()
        };
        let a_out = port(1, 100, "out");
        let a_in = port(2, 100, "in");
        let b_out = port(3, 200, "out");
        let b_in = port(4, 200, "in");
        assert!(
            validate(
                &registry.snapshot,
                a_out,
                b_in,
                Feedback::Reject,
                std::iter::empty()
            )
            .is_ok()
        );
        assert!(
            validate(
                &registry.snapshot,
                b_out,
                a_in,
                Feedback::Reject,
                [(100, 200)].into_iter()
            )
            .is_err()
        );
        assert!(
            validate(
                &registry.snapshot,
                b_out,
                a_in,
                Feedback::Allow,
                [(100, 200)].into_iter()
            )
            .is_ok()
        );
        assert!(
            validate(
                &registry.snapshot,
                a_out,
                a_in,
                Feedback::Reject,
                std::iter::empty()
            )
            .is_err()
        );
        registry.remove(1);
        assert_eq!(
            validate(
                &registry.snapshot,
                a_out,
                b_in,
                Feedback::Allow,
                std::iter::empty()
            ),
            Err(MediaError::StaleHandle)
        );
    }
}

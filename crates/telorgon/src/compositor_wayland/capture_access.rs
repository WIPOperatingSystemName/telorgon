//! Explicit connection-based access to direct capture and window discovery.
use super::NativeCompositorError;
use crate::wayland_server::{Display, NativeProtocol, OwnedClient};
use std::{
    cell::RefCell,
    os::unix::net::UnixStream,
    rc::{Rc, Weak},
};

const CAPTURE_GLOBALS: &[&str] = &[
    "ext_foreign_toplevel_list_v1",
    "ext_output_image_capture_source_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "ext_image_copy_capture_manager_v1",
];

/// Owner-thread capability for explicitly connected capture clients. This does not
/// advertise capture globals or grant ordinary socket clients access. Install before
/// creating protocol globals, then use the same policy for bind/request validation.
pub struct CaptureAccess {
    display: Rc<()>,
    clients: RefCell<Vec<Weak<OwnedClient>>>,
}

impl CaptureAccess {
    pub fn configure_display(display: &mut Display) -> Result<Rc<Self>, NativeCompositorError> {
        let interfaces = CAPTURE_GLOBALS
            .iter()
            .map(|name| {
                NativeProtocol::desktop()
                    .interface(name)
                    .map(|interface| interface as *const _)
                    .ok_or_else(|| NativeCompositorError::new(format!("missing {name}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let access = Rc::new(Self {
            display: display.identity_token(),
            clients: RefCell::new(Vec::new()),
        });
        let policy = access.clone();
        display.add_global_filter(move |client, interface| {
            !interfaces.contains(&interface) || policy.allows(client)
        });
        Ok(access)
    }

    /// Adopt a host-provided socket endpoint as a privileged capture connection.
    /// The caller retains the returned handle; dropping it disconnects the client.
    /// This grants screen and window-discovery access, independent of portal consent.
    pub fn create_client(
        &self,
        display: &Display,
        socket: UnixStream,
    ) -> Result<Rc<OwnedClient>, NativeCompositorError> {
        if !Rc::ptr_eq(&display.identity_token(), &self.display) {
            return Err(NativeCompositorError::new(
                "capture access belongs to another display",
            ));
        }
        let mut clients = self.clients.borrow_mut();
        clients.retain(|client| client.upgrade().is_some_and(|client| client.is_alive()));
        if clients.len() >= 8 {
            return Err(NativeCompositorError::new("capture client limit reached"));
        }
        let client = Rc::new(
            display
                .create_client(socket)
                .map_err(|error| NativeCompositorError::new(error.to_string()))?,
        );
        clients.push(Rc::downgrade(&client));
        Ok(client)
    }

    /// Revoke an authorized connection and destroy its bound protocol resources.
    pub fn revoke(&self, client: &Rc<OwnedClient>) {
        let mut clients = self.clients.borrow_mut();
        let old_len = clients.len();
        clients.retain(|candidate| !candidate.ptr_eq(&Rc::downgrade(client)));
        let authorized = clients.len() != old_len;
        drop(clients); // Native destruction callbacks may consult the policy.
        if authorized {
            client.disconnect();
        }
    }

    pub(crate) fn allows(&self, identity: usize) -> bool {
        self.clients.borrow().iter().any(|client| {
            client
                .upgrade()
                .is_some_and(|client| client.identity() == Some(identity))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_policy_denies_unknown_and_expired_connections() {
        let access = CaptureAccess {
            display: Rc::new(()),
            clients: RefCell::new(vec![Weak::new()]),
        };
        assert!(!access.allows(0));
        assert!(!access.allows(1));
        assert!(!access.allows(usize::MAX));
        for name in CAPTURE_GLOBALS {
            assert!(NativeProtocol::desktop().interface(name).is_some());
        }
    }
}

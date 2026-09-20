//! Trusted-client discovery; globals are enabled only by host capture policy.
use super::*;
use crate::integrations::wayland::compositor::foreign_toplevel::{
    Toplevel, ToplevelCatalog, ToplevelList,
};
use crate::shell::WindowId;
use std::collections::BTreeSet;
use std::ffi::CString;

const LIST: &str = "ext_foreign_toplevel_list_v1";
const HANDLE: &str = "ext_foreign_toplevel_handle_v1";

#[derive(Default)]
pub(super) struct NativeForeignToplevelState {
    catalog: ToplevelCatalog,
    pub lists: BTreeMap<ProtocolObjectId, ToplevelList>,
    pub handles: BTreeMap<ProtocolObjectId, NativeHandle>,
}

pub(super) struct NativeHandle {
    window: WindowId,
    last: Toplevel,
    closed: bool,
}

impl NativeState {
    pub(super) fn bind_foreign_toplevel_list(
        &mut self,
        resource: ResourceRef<'_>,
    ) -> Result<(), NativeCompositorError> {
        let id = self.protocol_object_for_resource(resource)?;
        self.foreign_toplevel
            .lists
            .insert(id, ToplevelList::default());
        self.refresh_foreign_toplevels()
    }

    pub(super) fn dispatch_foreign_toplevel_list(
        &mut self,
        resource: ResourceRef<'_>,
        context: &ResourceContext,
        request: &mut IncomingRequest<'_>,
    ) -> Result<DispatchOutcome, NativeCompositorError> {
        if request.message().name != "stop" {
            return Err(unsupported_request(request));
        }
        if self
            .foreign_toplevel
            .lists
            .get_mut(&context.object)
            .is_some_and(ToplevelList::stop)
        {
            self.post_event(resource, LIST, "finished", &mut [])?;
        }
        Ok(DispatchOutcome::default())
    }

    fn send_toplevel_metadata(
        &self,
        resource: ResourceRef<'_>,
        top: &Toplevel,
        initial: bool,
    ) -> Result<(), NativeCompositorError> {
        if initial {
            let identifier = CString::new(top.identifier()).map_err(error)?;
            self.post_event(
                resource,
                HANDLE,
                "identifier",
                &mut [ffi::wl_argument {
                    s: identifier.as_ptr(),
                }],
            )?;
        }
        for (event, text) in [("title", &top.title), ("app_id", &top.app_id)] {
            let text = CString::new(text.as_str()).map_err(error)?;
            self.post_event(
                resource,
                HANDLE,
                event,
                &mut [ffi::wl_argument { s: text.as_ptr() }],
            )?;
        }
        self.post_event(resource, HANDLE, "done", &mut [])
    }

    pub(super) fn refresh_foreign_toplevels(&mut self) -> Result<(), NativeCompositorError> {
        // Handles outlive their list and remain independently updateable until closed.
        let updates: Vec<_> = self
            .foreign_toplevel
            .handles
            .iter()
            .filter_map(|(&id, handle)| {
                if handle.closed {
                    return None;
                }
                let current = self
                    .foreign_toplevel
                    .catalog
                    .mapped
                    .get(&handle.window)
                    .filter(|top| top.epoch == handle.last.epoch);
                (current != Some(&handle.last)).then(|| (id, current.cloned()))
            })
            .collect();
        for (id, current) in updates {
            if let Some(resource) = self.resource_for_object(id)? {
                self.check_capture_access(
                    ResourceKind::ForeignToplevelHandle,
                    resource.client().identity(),
                )?;
                match &current {
                    Some(top) => self.send_toplevel_metadata(resource, top, false)?,
                    None => self.post_event(resource, HANDLE, "closed", &mut [])?,
                }
            }
            let handle = self
                .foreign_toplevel
                .handles
                .get_mut(&id)
                .expect("live handle");
            match current {
                Some(top) => handle.last = top,
                None => handle.closed = true,
            }
        }
        let tops: Vec<_> = self
            .foreign_toplevel
            .catalog
            .mapped
            .iter()
            .map(|(&window, top)| (window, top.clone()))
            .collect();
        let lists: Vec<_> = self.foreign_toplevel.lists.keys().copied().collect();
        for id in lists {
            // Owner-thread resource registry proves this pointer live. Event posting and
            // child creation do not dispatch client requests or destroy the parent.
            let Some(pointer) = self.resources.get(&id).copied() else {
                continue;
            };
            let Some(resource) =
                (unsafe { ResourceRef::from_raw(pointer as *mut ffi::wl_resource) })
            else {
                continue;
            };
            self.check_capture_access(
                ResourceKind::ForeignToplevelList,
                resource.client().identity(),
            )?;
            self.foreign_toplevel
                .lists
                .get_mut(&id)
                .expect("live list")
                .prune_unmapped(&self.foreign_toplevel.catalog);
            let client_id = self.ensure_client(resource.client())?;
            for (window, top) in &tops {
                if !self
                    .foreign_toplevel
                    .lists
                    .get_mut(&id)
                    .expect("live list")
                    .announce(top)
                {
                    continue;
                }
                let handle = self.create_resource(
                    resource.client(),
                    client_id,
                    HANDLE,
                    1,
                    0,
                    ResourceKind::ForeignToplevelHandle,
                    true,
                )?;
                let handle_id = self.protocol_object_for_resource(handle)?;
                self.foreign_toplevel.handles.insert(
                    handle_id,
                    NativeHandle {
                        window: *window,
                        last: top.clone(),
                        closed: false,
                    },
                );
                self.post_event(
                    resource,
                    LIST,
                    "toplevel",
                    &mut [ffi::wl_argument {
                        o: handle.identity() as *mut ffi::wl_resource,
                    }],
                )?;
                self.send_toplevel_metadata(handle, top, true)?;
            }
        }
        Ok(())
    }
}

impl NativeCompositor<'_> {
    /// Reconcile mapped windows, including mapped but minimized windows.
    pub(crate) fn sync_foreign_toplevels<'a>(
        &mut self,
        windows: impl IntoIterator<Item = (WindowId, &'a str, &'a str)>,
    ) -> Result<(), NativeCompositorError> {
        if self.state.capture_access.is_none() {
            return Ok(());
        }
        let mut live = BTreeSet::new();
        for (window, title, app_id) in windows {
            live.insert(window);
            self.state
                .foreign_toplevel
                .catalog
                .publish(window, title, app_id)
                .map_err(|()| {
                    NativeCompositorError::new("foreign toplevel catalog capacity exhausted")
                })?;
        }
        self.state
            .foreign_toplevel
            .catalog
            .mapped
            .retain(|window, _| live.contains(window));
        self.state.refresh_foreign_toplevels()
    }

    pub(crate) fn unmap_foreign_toplevel(
        &mut self,
        window: WindowId,
    ) -> Result<(), NativeCompositorError> {
        self.state.foreign_toplevel.catalog.unmap(window);
        self.state.refresh_foreign_toplevels()
    }
}

use super::*;

pub(super) unsafe extern "C" fn bind_global(
    client: *mut ffi::wl_client,
    data: *mut c_void,
    version: u32,
    id: u32,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let bind = unsafe { &mut *data.cast::<BindContext>() };
        let Some(client) = (unsafe { ClientRef::from_raw(client) }) else {
            return;
        };
        let state = unsafe { &mut *bind.state };
        if let Err(error) = state.bind(client, bind.interface, bind.kind, version, id) {
            if std::env::var("TELORGON_WAYLAND_ERROR_LOG").as_deref() == Ok("1") {
                eprintln!(
                    "telorgon-wayland-error: bind pid={} object={} version={} error={:?}",
                    client.credentials().pid,
                    id,
                    version,
                    error.to_string()
                );
            }
            client.post_no_memory();
        }
    }));
    if result.is_err() {
        // An unwind may never cross the C ABI. The client will be disconnected by libwayland when
        // its bind did not produce the requested object.
    }
}

fn log_rejected_request(resource: ResourceRef<'_>, interface: &str, opcode: u32, request: &str) {
    if std::env::var("TELORGON_WAYLAND_ERROR_LOG").as_deref() == Ok("1") {
        eprintln!(
            "telorgon-wayland-error: pid={} object={} interface={} opcode={} request={}",
            resource.client().credentials().pid,
            resource.id(),
            interface,
            opcode,
            request
        );
    }
}

pub(super) unsafe extern "C" fn dispatch_resource(
    _implementation: *const c_void,
    target: *mut c_void,
    opcode: u32,
    _message: *const ffi::wl_message,
    arguments: *mut ffi::wl_argument,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(resource) = (unsafe { ResourceRef::from_raw(target.cast::<ffi::wl_resource>()) })
        else {
            return -1;
        };
        let context_pointer = resource.user_data().cast::<ResourceContext>();
        if context_pointer.is_null() {
            return -1;
        }
        let (state_pointer, kind, interface) = {
            let context = unsafe { &*context_pointer };
            (context.state, context.kind, context.interface.clone())
        };
        let outcome = {
            let state = unsafe { &mut *state_pointer };
            let Some(message) = state
                .protocol
                .interface_schema(&interface)
                .and_then(|schema| schema.request(opcode))
                .cloned()
            else {
                log_rejected_request(resource, &interface, opcode, "unknown");
                resource.post_error(0, "unknown request opcode");
                return -1;
            };
            if message.since > resource.version() {
                log_rejected_request(resource, &interface, opcode, &message.name);
                resource.post_error(0, "request is newer than the bound interface version");
                return -1;
            }
            let mut request = match unsafe { IncomingRequest::from_raw(&message, arguments) } {
                Ok(request) => request,
                Err(error) => {
                    log_rejected_request(resource, &interface, opcode, &message.name);
                    resource.post_error(0, &error.to_string());
                    return -1;
                }
            };
            match state.dispatch(resource, context_pointer, kind, &mut request) {
                Ok(outcome) => {
                    state.collect_destroyed_buffers();
                    outcome
                }
                Err(error) => {
                    log_rejected_request(resource, &interface, opcode, &message.name);
                    resource.post_error(0, &error.to_string());
                    return -1;
                }
            }
        };
        for identity in outcome.destroy_others {
            if let Some(resource) =
                unsafe { ResourceRef::from_raw(identity as *mut ffi::wl_resource) }
            {
                unsafe { resource.destroy() };
            }
        }
        if outcome.destroy_self {
            unsafe { resource.destroy() };
        }
        0
    }));
    result.unwrap_or(-1)
}

pub(super) unsafe extern "C" fn destroy_resource(resource: *mut ffi::wl_resource) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(resource) = (unsafe { ResourceRef::from_raw(resource) }) else {
            return;
        };
        let pointer = resource.user_data().cast::<ResourceContext>();
        if pointer.is_null() {
            return;
        }
        unsafe { resource.set_user_data(std::ptr::null_mut()) };
        let context = unsafe { Box::from_raw(pointer) };
        let state = unsafe { &mut *context.state };
        state.destroy_context(&context);
    }));
}

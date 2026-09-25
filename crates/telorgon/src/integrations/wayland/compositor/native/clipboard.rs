use super::*;
use crate::platform::contracts::{ClipboardKind, DataFormat};
use crate::services::clipboard::{
    self as service, Clipboard, ClipboardContent, ClipboardError, ClipboardHost, ClipboardSnapshot,
    Command, ReadResponse,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

mod io;
#[cfg(test)]
mod tests;

pub(super) struct NativeClipboard {
    host: ClipboardHost,
    sources: [Option<ProtocolObjectId>; 2],
    formats: [Vec<DataFormat>; 2],
    revisions: [u64; 2],
    providers: BTreeMap<ProtocolObjectId, ClipboardContent>,
    jobs: Vec<Arc<AtomicBool>>,
    locked: bool,
}
impl Drop for NativeClipboard {
    fn drop(&mut self) {
        for job in &self.jobs {
            job.store(true, Ordering::Release);
        }
    }
}
impl NativeCompositor<'_> {
    pub(crate) fn install_clipboard(&mut self, wake: Arc<dyn Fn() + Send + Sync>) -> Clipboard {
        let host = ClipboardHost::new(wake);
        let handle = host.handle.clone();
        self.state.clipboard = Some(NativeClipboard {
            host,
            sources: [None; 2],
            formats: [vec![], vec![]],
            revisions: [1; 2],
            providers: BTreeMap::new(),
            jobs: vec![],
            locked: false,
        });
        handle
    }
    pub(crate) fn dispatch_clipboard(&mut self, locked: bool) {
        self.state.clipboard_turn(
            locked || self.state.active_session_lock.is_some() || self.state.secure_session_locked,
        );
    }
}
impl NativeState {
    pub(super) fn clipboard_turn(&mut self, locked: bool) {
        let Some(mut clipboard) = self.clipboard.take() else {
            return;
        };
        clipboard.locked = locked;
        let commands = clipboard.host.drain(locked);
        clipboard.jobs.retain(|job| Arc::strong_count(job) > 1);
        if locked {
            for job in &clipboard.jobs {
                job.store(true, Ordering::Release);
            }
        }
        let sources = [
            self.core.data_devices.selection(),
            self.core.data_devices.primary_selection(),
        ];
        for (slot, kind) in [ClipboardKind::System, ClipboardKind::Selection]
            .into_iter()
            .enumerate()
        {
            let source = sources[slot];
            let formats = source
                .and_then(|source| self.core.data_devices.source(source))
                .map(|source| {
                    source
                        .mime_types
                        .iter()
                        .filter_map(|mime| DataFormat::mime(mime.as_str()).ok())
                        .take(64)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if source != clipboard.sources[slot] || formats != clipboard.formats[slot] {
                clipboard.sources[slot] = source;
                clipboard.formats[slot] = formats;
                clipboard.revisions[slot] += 1;
            }
            clipboard.host.observe(ClipboardSnapshot {
                kind,
                revision: clipboard.revisions[slot],
                formats: clipboard.formats[slot].clone(),
            });
        }
        let retired: Vec<_> = clipboard
            .providers
            .keys()
            .copied()
            .filter(|id| !sources.contains(&Some(*id)))
            .collect();
        for id in retired {
            clipboard.providers.remove(&id);
            self.core.data_devices.remove_source(id);
        }
        self.clipboard = Some(clipboard);
        for command in commands {
            if locked {
                command.fail(ClipboardError::Denied);
                continue;
            }
            match command {
                Command::Publish(kind, content, expected, reply) => {
                    if reply.cancel.load(Ordering::Acquire) {
                        reply.finish(Err(ClipboardError::Cancelled));
                        continue;
                    }
                    let result = self.publish_clipboard(kind, Some(content), expected);
                    reply.finish(result);
                }
                Command::Clear(kind, expected, reply) => {
                    let result = if reply.cancel.load(Ordering::Acquire) {
                        Err(ClipboardError::Cancelled)
                    } else {
                        self.publish_clipboard(kind, None, expected)
                    };
                    reply.finish(result);
                }
                Command::Read(snapshot, format, max, reply) => {
                    self.read_clipboard(snapshot, format, max, reply)
                }
            }
        }
    }
    fn publish_clipboard(
        &mut self,
        kind: ClipboardKind,
        content: Option<ClipboardContent>,
        expected: Option<u64>,
    ) -> service::Result<()> {
        let slot = service::index(kind);
        let clipboard = self.clipboard.as_ref().unwrap();
        if expected.is_some_and(|revision| revision != clipboard.revisions[slot]) {
            return Err(ClipboardError::Stale);
        }
        let previous = if kind == ClipboardKind::System {
            self.core.data_devices.selection()
        } else {
            self.core.data_devices.primary_selection()
        };
        let owner = ClientId::from_raw(u32::MAX).unwrap();
        let source = if let Some(content) = content {
            let object = self
                .peek_next_object()
                .map_err(|_| ClipboardError::Unavailable)?;
            self.next_object = object.get();
            self.core
                .data_devices
                .create_source(crate::integrations::wayland::compositor::DataSource {
                    owner,
                    object,
                    mime_types: content
                        .formats()
                        .iter()
                        .map(|format| {
                            crate::integrations::wayland::compositor::MimeType::new(
                                format.identifier(),
                            )
                            .unwrap()
                        })
                        .collect(),
                    actions: crate::integrations::wayland::compositor::DataAction::NONE,
                    actions_set: false,
                    used: false,
                })
                .map_err(|_| ClipboardError::Unavailable)?;
            self.clipboard
                .as_mut()
                .unwrap()
                .providers
                .insert(object, content);
            Some(object)
        } else {
            None
        };
        if kind == ClipboardKind::System {
            self.core.data_devices.set_selection(owner, source)
        } else {
            self.core.data_devices.set_primary_selection(owner, source)
        }
        .map_err(|_| ClipboardError::Unavailable)?;
        if let Some(previous) = previous {
            let _ = self.cancel_data_source(previous);
            if self
                .clipboard
                .as_mut()
                .unwrap()
                .providers
                .remove(&previous)
                .is_some()
            {
                self.core.data_devices.remove_source(previous);
            }
        }
        let focused: Vec<_> = self
            .core
            .seats
            .iter()
            .filter_map(|(id, seat)| seat.keyboard_focus.map(|focus| (*id, focus.client)))
            .collect();
        for (seat, client) in focused {
            self.send_selection_to_client(seat, client)
                .map_err(|_| ClipboardError::TransferFailed)?;
        }
        let clipboard = self.clipboard.as_mut().unwrap();
        clipboard.sources[slot] = source;
        clipboard.formats[slot] = source
            .and_then(|id| clipboard.providers.get(&id))
            .map(|content| content.formats().to_vec())
            .unwrap_or_default();
        clipboard.revisions[slot] += 1;
        clipboard.host.observe(ClipboardSnapshot {
            kind,
            revision: clipboard.revisions[slot],
            formats: clipboard.formats[slot].clone(),
        });
        Ok(())
    }
    fn read_clipboard(
        &mut self,
        snapshot: ClipboardSnapshot,
        format: DataFormat,
        max: usize,
        reply: ReadResponse,
    ) {
        let clipboard = self.clipboard.as_mut().unwrap();
        let slot = service::index(snapshot.kind);
        if snapshot.revision != clipboard.revisions[slot] {
            reply.finish(Err(ClipboardError::Stale));
            return;
        }
        if clipboard.jobs.len() >= service::MAX_REQUESTS {
            reply.finish(Err(ClipboardError::Busy));
            return;
        }
        let Some(source) = clipboard.sources[slot] else {
            reply.finish(Err(ClipboardError::Empty));
            return;
        };
        if !clipboard.formats[slot].contains(&format) {
            reply.finish(Err(ClipboardError::InvalidFormat));
            return;
        }
        clipboard.jobs.push(reply.cancel());
        if let Some(content) = clipboard.providers.get(&source).cloned() {
            io::read_provider(content, format, max, reply);
            return;
        }
        let Ok((reader, writer)) = io::pipe() else {
            reply.finish(Err(ClipboardError::TransferFailed));
            return;
        };
        let sent = self.data_source_resource(source).and_then(|resource| {
            let mime = protocol_string(format.identifier());
            self.post_event(
                resource,
                self.source_interface(resource)?,
                "send",
                &mut [
                    ffi::wl_argument { s: mime.as_ptr() },
                    ffi::wl_argument {
                        h: writer.as_raw_fd(),
                    },
                ],
            )
        });
        drop(writer);
        if sent.is_err() {
            reply.finish(Err(ClipboardError::TransferFailed));
            return;
        }
        io::read_pipe(reader, max, reply);
    }
    pub(super) fn send_host_clipboard(
        &mut self,
        source: ProtocolObjectId,
        mime: &str,
        fd: OwnedFd,
    ) -> Result<bool, NativeCompositorError> {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return Ok(false);
        };
        let Some(content) = clipboard.providers.get(&source).cloned() else {
            return Ok(false);
        };
        if clipboard.locked || clipboard.jobs.len() >= service::MAX_REQUESTS {
            return Ok(true);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        clipboard.jobs.push(cancel.clone());
        let format = DataFormat::mime(mime).map_err(error)?;
        io::write_pipe(fd, content, format, cancel);
        Ok(true)
    }
}

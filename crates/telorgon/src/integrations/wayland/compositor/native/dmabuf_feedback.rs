//! Immutable, device-scoped DMA-BUF v4 allocation feedback. No image import or GPU work lives here.
use super::*;
use std::io::Write;

const INTERFACE: &str = "zwp_linux_dmabuf_feedback_v1";
const MAX_FORMATS: usize = u16::MAX as usize + 1;
// Keep each array event comfortably below libwayland's message-size limit.
const INDICES_PER_EVENT: usize = 512;

pub(super) struct DmaBufFeedback {
    main_device: libc::dev_t,
    table: std::fs::File,
    format_count: usize,
}

impl DmaBufFeedback {
    pub(super) fn new(
        main_device: libc::dev_t,
        formats: &[DmaBufFormat],
    ) -> Result<Self, NativeCompositorError> {
        if main_device == 0 || formats.is_empty() || formats.len() > MAX_FORMATS {
            return Err(NativeCompositorError::new(
                "invalid DMA-BUF feedback device or format count",
            ));
        }
        let raw = unsafe {
            libc::memfd_create(
                c"telorgon-dmabuf-formats".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if raw < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        let mut table = unsafe { std::fs::File::from_raw_fd(raw) };
        let mut bytes = Vec::with_capacity(formats.len() * 16);
        for format in formats {
            bytes.extend_from_slice(&format.fourcc.to_ne_bytes());
            bytes.extend_from_slice(&0_u32.to_ne_bytes());
            bytes.extend_from_slice(&format.modifier.to_ne_bytes());
        }
        table.write_all(&bytes).map_err(error)?;
        // Clients receive duplicated descriptors. Neither side may alter/truncate a published table.
        let seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(table.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(Self {
            main_device,
            table,
            format_count: formats.len(),
        })
    }
}

impl NativeState {
    pub(super) fn send_dmabuf_feedback(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<(), NativeCompositorError> {
        let feedback = self
            .dmabuf_feedback
            .as_ref()
            .ok_or_else(|| NativeCompositorError::new("DMA-BUF feedback is not configured"))?;
        self.post_event(
            resource,
            INTERFACE,
            "format_table",
            &mut [
                ffi::wl_argument {
                    h: feedback.table.as_raw_fd(),
                },
                ffi::wl_argument {
                    u: (feedback.format_count * 16) as u32,
                },
            ],
        )?;
        let mut device = feedback.main_device.to_ne_bytes();
        let mut device_array = ffi::wl_array {
            size: device.len(),
            alloc: 0,
            data: device.as_mut_ptr().cast(),
        };
        self.post_event(
            resource,
            INTERFACE,
            "main_device",
            &mut [ffi::wl_argument {
                a: &mut device_array,
            }],
        )?;
        self.post_event(
            resource,
            INTERFACE,
            "tranche_target_device",
            &mut [ffi::wl_argument {
                a: &mut device_array,
            }],
        )?;
        self.post_event(
            resource,
            INTERFACE,
            "tranche_flags",
            &mut [ffi::wl_argument { u: 0 }],
        )?;
        for first in (0..feedback.format_count).step_by(INDICES_PER_EVENT) {
            let end = (first + INDICES_PER_EVENT).min(feedback.format_count);
            let mut indices: Vec<u16> = (first..end).map(|index| index as u16).collect();
            let mut array = ffi::wl_array {
                size: indices.len() * std::mem::size_of::<u16>(),
                alloc: 0,
                data: indices.as_mut_ptr().cast(),
            };
            self.post_event(
                resource,
                INTERFACE,
                "tranche_formats",
                &mut [ffi::wl_argument { a: &mut array }],
            )?;
        }
        self.post_event(resource, INTERFACE, "tranche_done", &mut [])?;
        self.post_event(resource, INTERFACE, "done", &mut [])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::net::UnixStream, time::Duration};

    #[derive(Debug)]
    struct Event {
        object: u32,
        opcode: u16,
        body: Vec<u8>,
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect()
    }

    fn send(peer: &mut UnixStream, object: u32, opcode: u16, body: &[u8]) {
        peer.write_all(&words(&[
            object,
            ((body.len() + 8) as u32) << 16 | u32::from(opcode),
        ]))
        .unwrap();
        peer.write_all(body).unwrap();
    }

    // Decode the actual libwayland wire, including SCM_RIGHTS (FDs have no in-band payload).
    fn roundtrip(display: &Display, peer: &mut UnixStream) -> (Vec<Event>, Vec<OwnedFd>) {
        send(peer, 1, 0, &words(&[3]));
        display.dispatch_and_flush(Some(Duration::ZERO)).unwrap();
        let mut events = Vec::new();
        let mut fds = Vec::new();
        loop {
            let mut header = [0_u8; 8];
            let mut control = [0_usize; 32];
            let mut vector = libc::iovec {
                iov_base: header.as_mut_ptr().cast(),
                iov_len: header.len(),
            };
            let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
            message.msg_iov = &mut vector;
            message.msg_iovlen = 1;
            message.msg_control = control.as_mut_ptr().cast();
            message.msg_controllen = std::mem::size_of_val(&control);
            let read = unsafe {
                libc::recvmsg(
                    peer.as_raw_fd(),
                    &mut message,
                    libc::MSG_WAITALL | libc::MSG_CMSG_CLOEXEC,
                )
            };
            assert_eq!(read, 8, "{}", std::io::Error::last_os_error());
            assert_eq!(message.msg_flags & libc::MSG_CTRUNC, 0);
            unsafe {
                let mut ancillary = libc::CMSG_FIRSTHDR(&message);
                while !ancillary.is_null() {
                    assert_eq!((*ancillary).cmsg_level, libc::SOL_SOCKET);
                    assert_eq!((*ancillary).cmsg_type, libc::SCM_RIGHTS);
                    let count = ((*ancillary).cmsg_len - libc::CMSG_LEN(0) as usize)
                        / std::mem::size_of::<i32>();
                    let data = libc::CMSG_DATA(ancillary).cast::<i32>();
                    for index in 0..count {
                        fds.push(OwnedFd::from_raw_fd(*data.add(index)));
                    }
                    ancillary = libc::CMSG_NXTHDR(&message, ancillary);
                }
            }
            let object = u32::from_ne_bytes(header[..4].try_into().unwrap());
            let size_opcode = u32::from_ne_bytes(header[4..].try_into().unwrap());
            let mut body = vec![0; (size_opcode >> 16) as usize - 8];
            peer.read_exact(&mut body).unwrap();
            if object == 3 {
                return (events, fds);
            }
            events.push(Event {
                object,
                opcode: size_opcode as u16,
                body,
            });
        }
    }

    fn registry(display: &Display, peer: &mut UnixStream) -> BTreeMap<String, (u32, u32)> {
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        send(peer, 1, 1, &words(&[2]));
        let (events, fds) = roundtrip(display, peer);
        assert!(fds.is_empty());
        events
            .into_iter()
            .filter(|event| event.object == 2 && event.opcode == 0)
            .map(|event| {
                let name = u32::from_ne_bytes(event.body[..4].try_into().unwrap());
                let len = u32::from_ne_bytes(event.body[4..8].try_into().unwrap()) as usize;
                let interface = String::from_utf8(event.body[8..8 + len - 1].to_vec()).unwrap();
                let version =
                    u32::from_ne_bytes(event.body[event.body.len() - 4..].try_into().unwrap());
                (interface, (name, version))
            })
            .collect()
    }

    fn bind(
        peer: &mut UnixStream,
        globals: &BTreeMap<String, (u32, u32)>,
        interface: &str,
        id: u32,
        version: u32,
    ) {
        let mut body = words(&[globals[interface].0, interface.len() as u32 + 1]);
        body.extend_from_slice(interface.as_bytes());
        body.push(0);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        body.extend(words(&[version, id]));
        send(peer, 2, 0, &body);
    }

    fn formats() -> Vec<DmaBufFormat> {
        vec![
            DmaBufFormat {
                fourcc: 0x34325258,
                modifier: 0x0200_0000_0000_0001,
            },
            DmaBufFormat {
                fourcc: 0x34325241,
                modifier: 0,
            },
            DmaBufFormat {
                fourcc: 0x34325241,
                modifier: 0,
            },
        ]
    }

    fn array(body: &[u8]) -> &[u8] {
        let len = u32::from_ne_bytes(body[..4].try_into().unwrap()) as usize;
        &body[4..4 + len]
    }

    #[test]
    fn dmabuf_v4_feedback_wire_and_lifetime() {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let device = libc::makedev(226, 129);
        native
            .add_linux_dmabuf_with_feedback(&display, formats(), device)
            .unwrap();
        let globals = registry(&display, &mut peer);
        assert_eq!(globals["zwp_linux_dmabuf_v1"].1, 4);
        bind(&mut peer, &globals, "zwp_linux_dmabuf_v1", 4, 4);
        bind(&mut peer, &globals, "wl_compositor", 5, 4);
        send(&mut peer, 5, 0, &words(&[6])); // wl_surface
        let (events, fds) = roundtrip(&display, &mut peer);
        assert!(
            !events.iter().any(|event| event.object == 4),
            "v4 must not send legacy format/modifier events"
        );
        assert!(fds.is_empty());
        send(&mut peer, 4, 2, &words(&[7])); // default feedback
        send(&mut peer, 4, 3, &words(&[8, 6])); // surface feedback
        let (events, fds) = roundtrip(&display, &mut peer);
        assert_eq!(fds.len(), 2);
        for id in [7, 8] {
            let feedback: Vec<_> = events.iter().filter(|event| event.object == id).collect();
            assert_eq!(
                feedback
                    .iter()
                    .map(|event| event.opcode)
                    .collect::<Vec<_>>(),
                [1, 2, 4, 6, 5, 3, 0]
            );
            assert_eq!(feedback[0].body, words(&[32])); // two deduplicated 16-byte table entries
            assert_eq!(array(&feedback[1].body), device.to_ne_bytes());
            assert_eq!(array(&feedback[2].body), device.to_ne_bytes());
            assert_eq!(feedback[3].body, words(&[0])); // no direct-scanout claim
            assert_eq!(
                array(&feedback[4].body),
                [0_u16, 1]
                    .into_iter()
                    .flat_map(u16::to_ne_bytes)
                    .collect::<Vec<_>>()
            );
        }
        for fd in fds {
            let table = std::fs::File::from(fd);
            let mut bytes = [0_u8; 32];
            table.read_exact_at(&mut bytes, 0).unwrap();
            assert_eq!(&bytes[..4], &0x34325241_u32.to_ne_bytes());
            assert_eq!(&bytes[4..16], &[0; 12]);
            assert_eq!(&bytes[16..20], &0x34325258_u32.to_ne_bytes());
            assert_eq!(&bytes[24..], &0x0200_0000_0000_0001_u64.to_ne_bytes());
            assert!(table.write_at(&[1], 0).is_err());
            assert!(table.set_len(0).is_err());
        }
        send(&mut peer, 4, 1, &words(&[9])); // params inherit v4 and survive factory destruction
        send(&mut peer, 6, 0, &[]); // surface feedback becomes inert
        send(&mut peer, 4, 0, &[]);
        roundtrip(&display, &mut peer);
        assert!(client.is_alive());
        send(&mut peer, 7, 0, &[]);
        send(&mut peer, 8, 0, &[]);
        send(&mut peer, 9, 0, &[]);
        roundtrip(&display, &mut peer);
        assert!(client.is_alive());
        assert!(native.state.dmabuf_params.is_empty());
        assert!(!native.state.resources.values().any(|raw| {
            let resource = unsafe { ResourceRef::from_raw(*raw as *mut ffi::wl_resource).unwrap() };
            matches!(
                native.state.resource_kind(resource).unwrap(),
                ResourceKind::LinuxDmaBufFeedback
            )
        }));
    }

    #[test]
    fn dmabuf_v3_bind_preserves_modifier_advertisement() {
        for feedback in [false, true] {
            let display = Display::new().unwrap();
            let (mut peer, socket) = UnixStream::pair().unwrap();
            let client = display.create_client(socket).unwrap();
            let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
            if feedback {
                native
                    .add_linux_dmabuf_with_feedback(&display, formats(), libc::makedev(226, 1))
                    .unwrap();
            } else {
                native.add_linux_dmabuf(&display, formats()).unwrap();
            }
            let globals = registry(&display, &mut peer);
            assert_eq!(
                globals["zwp_linux_dmabuf_v1"].1,
                if feedback { 4 } else { 3 }
            );
            bind(&mut peer, &globals, "zwp_linux_dmabuf_v1", 4, 3);
            let (events, fds) = roundtrip(&display, &mut peer);
            assert!(fds.is_empty());
            let modifiers: Vec<_> = events.iter().filter(|event| event.object == 4).collect();
            assert_eq!(modifiers.len(), 2);
            assert!(modifiers.iter().all(|event| event.opcode == 1));
            assert_eq!(modifiers[0].body, words(&[0x34325241, 0, 0]));
            assert_eq!(modifiers[1].body, words(&[0x34325258, 0x0200_0000, 1]));
            assert!(client.is_alive());
        }
    }

    #[test]
    fn dmabuf_feedback_rejects_invalid_configuration_atomically() {
        let display = Display::new().unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let device = libc::makedev(226, 1);
        assert!(
            native
                .add_linux_dmabuf_with_feedback(&display, formats(), 0)
                .is_err()
        );
        assert!(
            native
                .add_linux_dmabuf_with_feedback(&display, Vec::new(), device)
                .is_err()
        );
        assert!(native.state.dmabuf_formats.is_empty());
        let too_many: Vec<_> = (0..=MAX_FORMATS)
            .map(|i| DmaBufFormat {
                fourcc: 1,
                modifier: i as u64,
            })
            .collect();
        assert!(
            native
                .add_linux_dmabuf_with_feedback(&display, too_many, device)
                .is_err()
        );
        assert!(native.state.dmabuf_feedback.is_none());
        native
            .add_linux_dmabuf_with_feedback(&display, formats(), device)
            .unwrap();
        assert!(native.add_linux_dmabuf(&display, formats()).is_err());
        assert!(
            native
                .add_linux_dmabuf_with_feedback(&display, formats(), libc::makedev(226, 2))
                .is_err()
        );
        assert_eq!(
            native.state.dmabuf_feedback.as_ref().unwrap().main_device,
            device
        );
    }

    #[test]
    fn dmabuf_feedback_splits_large_index_arrays() {
        let display = Display::new().unwrap();
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let client = display.create_client(socket).unwrap();
        let mut native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let count = INDICES_PER_EVENT * 2 + 1;
        let formats = (0..count)
            .map(|i| DmaBufFormat {
                fourcc: 1,
                modifier: i as u64,
            })
            .collect();
        native
            .add_linux_dmabuf_with_feedback(&display, formats, libc::makedev(226, 1))
            .unwrap();
        let globals = registry(&display, &mut peer);
        bind(&mut peer, &globals, "zwp_linux_dmabuf_v1", 4, 4);
        send(&mut peer, 4, 2, &words(&[5]));
        let (events, fds) = roundtrip(&display, &mut peer);
        assert_eq!(fds.len(), 1);
        let arrays: Vec<_> = events
            .iter()
            .filter(|event| event.object == 5 && event.opcode == 5)
            .collect();
        assert_eq!(arrays.len(), 3);
        let indices: Vec<_> = arrays
            .iter()
            .flat_map(|event| {
                array(&event.body)
                    .chunks_exact(2)
                    .map(|bytes| u16::from_ne_bytes(bytes.try_into().unwrap()))
            })
            .collect();
        assert_eq!(indices, (0..count as u16).collect::<Vec<_>>());
        assert!(client.is_alive());
    }
}

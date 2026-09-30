use std::{
    mem::{offset_of, size_of},
    os::unix::net::UnixDatagram,
    sync::mpsc,
    thread,
    time::Duration,
};

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(2);
const DEVICE_PATH: &str = "/devices/platform/ACPI0003:00/power_supply/AC";

fn event(action: &str, subsystem: &str) -> Vec<u8> {
    format!(
        "{action}@{DEVICE_PATH}\0ACTION={action}\0DEVPATH={DEVICE_PATH}\0SUBSYSTEM={subsystem}\0"
    )
    .into_bytes()
}

fn sender() -> libc::sockaddr_nl {
    // SAFETY: sockaddr_nl has a valid zero representation, including its reserved padding.
    let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    address.nl_groups = KERNEL_GROUP;
    address
}

fn credentials(pid: libc::pid_t, uid: libc::uid_t, gid: libc::gid_t) -> Vec<u8> {
    let header_size = size_of::<libc::cmsghdr>();
    let length = header_size + size_of::<libc::ucred>();
    let alignment = size_of::<usize>();
    let mut control = vec![0; (length + alignment - 1) & !(alignment - 1)];
    write_field(
        &mut control,
        offset_of!(libc::cmsghdr, cmsg_len),
        &length.to_ne_bytes(),
    );
    write_field(
        &mut control,
        offset_of!(libc::cmsghdr, cmsg_level),
        &libc::SOL_SOCKET.to_ne_bytes(),
    );
    write_field(
        &mut control,
        offset_of!(libc::cmsghdr, cmsg_type),
        &libc::SCM_CREDENTIALS.to_ne_bytes(),
    );
    write_field(
        &mut control,
        header_size + offset_of!(libc::ucred, pid),
        &pid.to_ne_bytes(),
    );
    write_field(
        &mut control,
        header_size + offset_of!(libc::ucred, uid),
        &uid.to_ne_bytes(),
    );
    write_field(
        &mut control,
        header_size + offset_of!(libc::ucred, gid),
        &gid.to_ne_bytes(),
    );
    control
}

fn write_field(buffer: &mut [u8], offset: usize, value: &[u8]) {
    buffer[offset..offset + value.len()].copy_from_slice(value);
}

fn authenticated(address: &libc::sockaddr_nl, flags: libc::c_int, control: &[u8]) -> bool {
    kernel_sender(
        address,
        size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        flags,
        control,
    )
}

#[test]
fn battery_and_external_power_changes_additions_and_removals_are_refresh_hints() {
    for action in ["change", "add", "remove"] {
        let mut payload = event(action, "power_supply");
        // Device details are opaque hints: no attribute or path from a packet is a reading.
        payload.extend_from_slice(b"POWER_SUPPLY_NAME=AC\0POWER_SUPPLY_ONLINE=1\0");
        assert!(power_supply_event(&payload), "rejected {action}");
    }
    assert!(!power_supply_event(&event("change", "input")));
    assert!(!power_supply_event(&event("bind", "power_supply")));
}

#[test]
fn incomplete_conflicting_or_ambiguous_payloads_are_ignored() {
    let valid = event("change", "power_supply");
    let mut unterminated = valid.clone();
    unterminated.pop();
    let mut duplicate_action = valid.clone();
    duplicate_action.extend_from_slice(b"ACTION=change\0");
    let mut duplicate_path = valid.clone();
    duplicate_path.extend_from_slice(format!("DEVPATH={DEVICE_PATH}\0").as_bytes());
    let mut duplicate_subsystem = valid.clone();
    duplicate_subsystem.extend_from_slice(b"SUBSYSTEM=power_supply\0");
    let malformed = [
        ("unterminated", unterminated),
        ("duplicate action", duplicate_action),
        ("duplicate path", duplicate_path),
        ("duplicate subsystem", duplicate_subsystem),
        ("empty payload", Vec::new()),
        (
            "missing action",
            format!("change@{DEVICE_PATH}\0DEVPATH={DEVICE_PATH}\0SUBSYSTEM=power_supply\0")
                .into_bytes(),
        ),
        (
            "missing path",
            format!("change@{DEVICE_PATH}\0ACTION=change\0SUBSYSTEM=power_supply\0")
                .into_bytes(),
        ),
        (
            "missing subsystem",
            format!("change@{DEVICE_PATH}\0ACTION=change\0DEVPATH={DEVICE_PATH}\0")
                .into_bytes(),
        ),
        (
            "conflicting action",
            format!("change@{DEVICE_PATH}\0ACTION=remove\0DEVPATH={DEVICE_PATH}\0SUBSYSTEM=power_supply\0")
                .into_bytes(),
        ),
        (
            "conflicting path",
            format!("change@{DEVICE_PATH}\0ACTION=change\0DEVPATH=/devices/other\0SUBSYSTEM=power_supply\0")
                .into_bytes(),
        ),
        (
            "invalid header",
            format!("{DEVICE_PATH}\0ACTION=change\0DEVPATH={DEVICE_PATH}\0SUBSYSTEM=power_supply\0")
                .into_bytes(),
        ),
        (
            "nondevice path",
            b"change@/tmp/battery\0ACTION=change\0DEVPATH=/tmp/battery\0SUBSYSTEM=power_supply\0".to_vec(),
        ),
        ("empty field", [valid.as_slice(), b"\0"].concat()),
        ("field without equals", [valid.as_slice(), b"unexpected\0"].concat()),
    ];
    for (reason, payload) in malformed {
        assert!(!power_supply_event(&payload), "accepted {reason}");
    }
}

#[test]
fn sender_must_be_kernel_multicast_with_complete_zero_credentials() {
    let address = sender();
    let valid = credentials(0, 0, 0);
    assert!(authenticated(&address, 0, &valid));
    // Truncated payloads still require authentication before they can request recovery.
    assert!(authenticated(&address, libc::MSG_TRUNC, &valid));
    assert!(!authenticated(&address, libc::MSG_CTRUNC, &valid));
    assert!(!authenticated(&address, 0, &[]));
    for control in [
        credentials(123, 0, 0),
        credentials(0, 1000, 0),
        credentials(0, 0, 1000),
    ] {
        assert!(!authenticated(&address, 0, &control));
    }
    for length in [
        0,
        size_of::<libc::sockaddr_nl>() - 1,
        size_of::<libc::sockaddr_nl>() + 1,
    ] {
        assert!(!kernel_sender(
            &address,
            length as libc::socklen_t,
            0,
            &valid
        ));
    }
    let mut userspace = sender();
    userspace.nl_pid = 123;
    assert!(!authenticated(&userspace, 0, &valid));
    let mut unicast = sender();
    unicast.nl_groups = 0;
    assert!(!authenticated(&unicast, 0, &valid));
    let mut other_group = sender();
    other_group.nl_groups = 2;
    assert!(!authenticated(&other_group, 0, &valid));
    let mut wrong_family = sender();
    wrong_family.nl_family = libc::AF_UNIX as libc::sa_family_t;
    assert!(!authenticated(&wrong_family, 0, &valid));
}

#[test]
fn malformed_or_duplicate_ancillary_credentials_are_rejected() {
    let address = sender();
    let valid = credentials(0, 0, 0);
    let credential_length = size_of::<libc::cmsghdr>() + size_of::<libc::ucred>();
    for length in [
        0,
        size_of::<libc::cmsghdr>() - 1,
        credential_length - 1,
        usize::MAX,
    ] {
        let mut control = valid.clone();
        write_field(
            &mut control,
            offset_of!(libc::cmsghdr, cmsg_len),
            &length.to_ne_bytes(),
        );
        assert!(
            !authenticated(&address, 0, &control),
            "accepted length {length}"
        );
    }
    assert!(!authenticated(&address, 0, &valid[..credential_length - 1]));
    assert!(!authenticated(
        &address,
        0,
        &[valid.as_slice(), valid.as_slice()].concat()
    ));
    let mut wrong_level = valid.clone();
    write_field(
        &mut wrong_level,
        offset_of!(libc::cmsghdr, cmsg_level),
        &0_i32.to_ne_bytes(),
    );
    assert!(!authenticated(&address, 0, &wrong_level));
    let mut wrong_type = valid.clone();
    write_field(
        &mut wrong_type,
        offset_of!(libc::cmsghdr, cmsg_type),
        &libc::SCM_RIGHTS.to_ne_bytes(),
    );
    assert!(!authenticated(&address, 0, &wrong_type));
    // The last message may omit alignment padding, but unexplained bytes after that
    // message's full padding must not hide a malformed second control message.
    assert!(authenticated(&address, 0, &valid[..credential_length]));
    assert!(!authenticated(
        &address,
        0,
        &[valid.as_slice(), &[0x7f]].concat()
    ));
}

#[test]
fn packet_loss_requests_recovery_only_after_kernel_authentication() {
    let address = sender();
    let length = size_of::<libc::sockaddr_nl>() as libc::socklen_t;
    let control = credentials(0, 0, 0);
    let payload = event("change", "power_supply");
    assert_eq!(
        classify_message(&address, length, 0, &control, &payload),
        Some(Notification::Changed)
    );
    let incomplete_payload = b"change@/devices/platform/";
    assert_eq!(
        classify_message(
            &address,
            length,
            libc::MSG_TRUNC,
            &control,
            incomplete_payload,
        ),
        Some(Notification::Resync)
    );
    let mut forged = sender();
    forged.nl_pid = 123;
    assert_eq!(
        classify_message(&forged, length, libc::MSG_TRUNC, &control, &payload),
        None
    );
    assert_eq!(
        classify_message(
            &address,
            length,
            libc::MSG_TRUNC,
            &credentials(123, 0, 0),
            &payload,
        ),
        None
    );
    assert_eq!(
        classify_message(&address, length, libc::MSG_TRUNC, &[], &payload),
        None
    );
    assert_eq!(
        classify_message(
            &address,
            length,
            libc::MSG_TRUNC | libc::MSG_CTRUNC,
            &control,
            &payload,
        ),
        None
    );
    assert_eq!(
        classify_message(&address, length, 0, &control, &event("change", "input")),
        None
    );
}

#[test]
fn shutdown_descriptor_is_nonblocking_and_does_not_leak_through_exec() {
    let wake = ShutdownWake::new().unwrap();
    // SAFETY: fcntl reads scalar flags from the owned, live eventfd.
    let status = unsafe { libc::fcntl(wake.0.as_raw_fd(), libc::F_GETFL) };
    assert!(status >= 0, "{}", io::Error::last_os_error());
    assert_ne!(status & libc::O_NONBLOCK, 0);
    // SAFETY: the owned eventfd remains open during this descriptor flag query.
    let descriptor = unsafe { libc::fcntl(wake.0.as_raw_fd(), libc::F_GETFD) };
    assert!(descriptor >= 0, "{}", io::Error::last_os_error());
    assert_ne!(descriptor & libc::FD_CLOEXEC, 0);
}

fn poll_in_worker(
    socket: OwnedFd,
    shutdown: Arc<ShutdownWake>,
    trigger: impl FnOnce(),
) -> io::Result<bool> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (completed_tx, completed_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        ready_tx.send(()).unwrap();
        let _ = completed_tx.send(wait_readable(&socket, &shutdown));
    });
    ready_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("poll worker did not start");
    trigger();
    let result = completed_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("poll worker did not wake within the bounded test wait");
    worker.join().unwrap();
    result
}

#[test]
fn socket_readiness_wakes_the_worker_without_a_timer() {
    let (reader, writer) = UnixDatagram::pair().unwrap();
    let wake = Arc::new(ShutdownWake::new().unwrap());
    assert!(
        poll_in_worker(reader.into(), wake, || {
            writer.send(b"notification").unwrap();
        })
        .unwrap()
    );
}

#[test]
fn shutdown_wakes_an_idle_worker_and_wins_over_pending_notifications() {
    let (reader, _writer) = UnixDatagram::pair().unwrap();
    let wake = Arc::new(ShutdownWake::new().unwrap());
    let trigger = wake.clone();
    assert!(!poll_in_worker(reader.into(), wake, || trigger.notify()).unwrap());

    let (reader, writer) = UnixDatagram::pair().unwrap();
    let wake = Arc::new(ShutdownWake::new().unwrap());
    writer.send(b"pending notification").unwrap();
    wake.notify();
    wake.notify();
    assert!(!poll_in_worker(reader.into(), wake, || {}).unwrap());
}

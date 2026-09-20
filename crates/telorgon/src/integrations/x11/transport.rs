//! Nonblocking framing for the private XWM socket.
//!
//! The initial XWM request set has no FD-bearing replies. Unexpected ancillary
//! descriptors are closed and rejected, not silently admitted. Request/reply
//! policy and deadlines belong to the XWM adapter above this transport.
use super::{Error, Result};
use std::{
    collections::VecDeque,
    io::{self, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    time::{Duration, Instant},
};
use x11rb_protocol::{connect::Connect, protocol::xproto::Setup};

const MAX_PACKET: usize = 256 * 1024;
const MAX_OUTGOING: usize = 1024 * 1024;
const TURN_BYTES: usize = 256 * 1024;
const TURN_PACKETS: usize = 256;
const TURN_TIME: Duration = Duration::from_millis(1);

pub struct Turn {
    pub setup: Option<Setup>,
    pub packets: Vec<Vec<u8>>,
    /// Schedule another turn without waiting for a new edge-triggered event.
    pub reschedule: bool,
    pub writable_interest: bool,
}
pub struct Transport {
    stream: UnixStream,
    handshake: Option<Connect>,
    outgoing: VecDeque<Vec<u8>>,
    sent: usize,
    queued: usize,
    incoming: Vec<u8>,
    expected: usize,
    failed: bool,
}
impl Transport {
    /// The supplied socket must be the privately inherited XWM connection.
    /// This does not connect to DISPLAY or search for any external X server.
    pub fn new(stream: UnixStream) -> Result<Self> {
        stream.set_nonblocking(true)?;
        let (handshake, request) = Connect::with_authorization(Vec::new(), Vec::new());
        let queued = request.len();
        Ok(Self {
            stream,
            handshake: Some(handshake),
            outgoing: VecDeque::from([request]),
            sent: 0,
            queued,
            incoming: Vec::new(),
            expected: 32,
            failed: false,
        })
    }
    pub fn fd(&self) -> std::os::fd::RawFd {
        self.stream.as_raw_fd()
    }
    pub fn wants_write(&self) -> bool {
        self.queued != 0
    }
    /// Queue a normal (non-BIG-REQUESTS) serialized request without blocking.
    /// Call only after setup; backpressure leaves the queue unchanged.
    pub fn queue(&mut self, request: Vec<u8>) -> Result<()> {
        self.check_queue(&request)?;
        self.queued += request.len();
        self.outgoing.push_back(request);
        Ok(())
    }
    pub(crate) fn check_queue(&self, request: &[u8]) -> Result<()> {
        if self.failed || self.handshake.is_some() {
            return Err(Error("XWM transport is not ready".into()));
        }
        if request.len() < 4
            || request.len() > u16::MAX as usize * 4
            || request.len() != u16::from_ne_bytes([request[2], request[3]]) as usize * 4
        {
            return Err(Error("invalid XWM request framing".into()));
        }
        if request.len() > MAX_OUTGOING - self.queued {
            return Err(Error("XWM output backpressure".into()));
        }
        Ok(())
    }
    /// Drive at most 256 packets, 256 KiB of socket I/O, or one millisecond.
    /// No polling, synchronous replies or blocking flushes occur here.
    pub fn dispatch(&mut self) -> Result<Turn> {
        if self.failed {
            return Err(Error("XWM transport has failed".into()));
        }
        let result = self.dispatch_inner();
        if result.is_err() {
            self.failed = true;
            self.outgoing.clear();
            self.incoming.clear();
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        result
    }
    fn dispatch_inner(&mut self) -> Result<Turn> {
        let started = Instant::now();
        let mut turn = Turn {
            setup: None,
            packets: Vec::new(),
            reschedule: false,
            writable_interest: false,
        };
        let (mut bytes, mut operations) = (0, 0);
        while let Some(front) = self.outgoing.front() {
            if bytes >= TURN_BYTES / 2 || operations >= 256 || started.elapsed() >= TURN_TIME {
                turn.reschedule = true;
                break;
            }
            operations += 1;
            let end = front.len().min(self.sent + TURN_BYTES / 2 - bytes);
            match self.stream.write(&front[self.sent..end]) {
                Ok(0) => return Err(Error("XWM socket stopped accepting writes".into())),
                Ok(n) => {
                    self.sent += n;
                    self.queued -= n;
                    bytes += n;
                    if self.sent == front.len() {
                        self.outgoing.pop_front();
                        self.sent = 0;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        loop {
            if bytes >= TURN_BYTES
                || operations >= 512
                || turn.packets.len() >= TURN_PACKETS
                || started.elapsed() >= TURN_TIME
            {
                turn.reschedule = true;
                break;
            }
            operations += 1;
            if let Some(handshake) = self.handshake.as_mut() {
                let buffer = handshake.buffer();
                if buffer.is_empty() {
                    // A zero-length failure payload completes after the header.
                    let setup = self
                        .handshake
                        .take()
                        .unwrap()
                        .into_setup()
                        .map_err(|_| Error("XWM setup rejected or malformed".into()))?;
                    turn.setup = Some(setup);
                    continue;
                }
                let length = buffer.len().min(TURN_BYTES - bytes);
                match receive(&self.stream, &mut buffer[..length]) {
                    Ok(0) => return Err(Error("XWM disconnected during setup".into())),
                    Ok(n) => {
                        bytes += n;
                        if handshake.advance(n) {
                            turn.setup =
                                Some(self.handshake.take().unwrap().into_setup().map_err(
                                    |_| Error("XWM setup rejected or malformed".into()),
                                )?);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
                continue;
            }
            let needed = (self.expected - self.incoming.len())
                .min(TURN_BYTES - bytes)
                .min(8192);
            let mut scratch = [0u8; 8192];
            match receive(&self.stream, &mut scratch[..needed]) {
                Ok(0) => return Err(Error("XWM disconnected".into())),
                Ok(n) => {
                    bytes += n;
                    self.incoming.extend_from_slice(&scratch[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
            if self.incoming.len() == 32 && matches!(self.incoming[0] & 0x7f, 1 | 35) {
                let extra = u32::from_ne_bytes(self.incoming[4..8].try_into().unwrap()) as usize;
                if extra > (MAX_PACKET - 32) / 4 {
                    return Err(Error("oversized XWM packet".into()));
                }
                self.expected = 32 + extra * 4;
            }
            if self.incoming.len() == self.expected {
                turn.packets.push(std::mem::take(&mut self.incoming));
                self.expected = 32;
            }
        }
        turn.writable_interest = !self.outgoing.is_empty();
        Ok(turn)
    }
}

fn receive(stream: &UnixStream, buffer: &mut [u8]) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: buffer.as_mut_ptr().cast(),
        iov_len: buffer.len(),
    };
    // usize alignment is sufficient for Linux cmsghdr. The kernel owns neither
    // buffer after recvmsg returns. MSG_CMSG_CLOEXEC closes the exec race.
    let mut control = [0usize; 32];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = std::mem::size_of_val(&control);
    let count = unsafe {
        libc::recvmsg(
            stream.as_raw_fd(),
            &mut msg,
            libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
        )
    };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut unexpected = msg.msg_flags & libc::MSG_CTRUNC != 0;
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&msg);
        while !header.is_null() {
            unexpected = true;
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let length = (*header)
                    .cmsg_len
                    .saturating_sub(libc::CMSG_LEN(0) as usize);
                let data = libc::CMSG_DATA(header).cast::<libc::c_int>();
                for i in 0..length / std::mem::size_of::<libc::c_int>() {
                    libc::close(*data.add(i));
                }
            }
            header = libc::CMSG_NXTHDR(&msg, header);
        }
    }
    if unexpected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected XWM ancillary data",
        ));
    }
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> (Transport, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        let mut wire = Transport::new(a).unwrap();
        // Isolate packet framing from setup parsing in these socket-pair tests.
        wire.handshake = None;
        wire.outgoing.clear();
        wire.queued = 0;
        (wire, b)
    }
    #[test]
    fn partial_headers_and_bodies_remain_buffered() {
        let (mut wire, mut server) = ready();
        let mut reply = vec![0; 36];
        reply[0] = 1;
        reply[4..8].copy_from_slice(&1u32.to_ne_bytes());
        server.write_all(&reply[..9]).unwrap();
        assert!(wire.dispatch().unwrap().packets.is_empty());
        server.write_all(&reply[9..34]).unwrap();
        assert!(wire.dispatch().unwrap().packets.is_empty());
        server.write_all(&reply[34..]).unwrap();
        assert_eq!(wire.dispatch().unwrap().packets, vec![reply]);
    }
    #[test]
    fn oversized_packet_fails_only_the_transport() {
        let (mut wire, mut server) = ready();
        let mut reply = [0; 32];
        reply[0] = 1;
        reply[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
        server.write_all(&reply).unwrap();
        assert!(wire.dispatch().is_err());
        assert!(wire.dispatch().is_err());
    }
    #[test]
    fn stalled_writer_has_bounded_queue() {
        let (mut wire, _server) = ready();
        let send_size: libc::c_int = (TURN_BYTES * 4) as libc::c_int;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    wire.fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&send_size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&send_size) as libc::socklen_t,
                )
            },
            0
        );
        let mut request = vec![0; 65536];
        request[2..4].copy_from_slice(&16384u16.to_ne_bytes());
        for _ in 0..16 {
            wire.queue(request.clone()).unwrap();
        }
        assert!(wire.queue(request).is_err());
        let turn = wire.dispatch().unwrap();
        assert!(turn.writable_interest);
        assert!(
            turn.reschedule,
            "output-budget exhaustion must not wait for another writable edge"
        );
        assert!(wire.queued <= MAX_OUTGOING);
    }
    #[test]
    fn event_storm_yields_and_disconnect_is_terminal() {
        let (mut wire, mut server) = ready();
        let mut event = [0; 32];
        event[0] = 12;
        server.write_all(&event.repeat(300)).unwrap();
        let turn = wire.dispatch().unwrap();
        assert!(turn.packets.len() <= TURN_PACKETS);
        assert!(turn.reschedule);
        drop(server);
        assert!(wire.dispatch().is_err());
    }
    #[test]
    fn setup_failure_does_not_log_server_supplied_text() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut wire = Transport::new(a).unwrap();
        let mut failed = vec![0, 4, 11, 0, 0, 0, 1, 0];
        failed[6..8].copy_from_slice(&1u16.to_ne_bytes());
        failed.extend(b"text");
        b.write_all(&failed).unwrap();
        let error = wire.dispatch().err().unwrap().to_string();
        assert!(!error.contains("text"));
    }
    #[test]
    fn unexpected_rights_are_closed_before_failure() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let (mut wire, server) = ready();
        let mut pipe = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) },
            0
        );
        let reader = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
        let mut event = [12u8; 32];
        let mut iov = libc::iovec {
            iov_base: event.as_mut_ptr().cast(),
            iov_len: event.len(),
        };
        let mut control = [0usize; 8];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) } as usize;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&msg);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as usize;
            *libc::CMSG_DATA(header).cast::<i32>() = writer.as_raw_fd();
            assert_eq!(
                libc::sendmsg(server.as_raw_fd(), &msg, libc::MSG_NOSIGNAL),
                32
            );
        }
        drop(writer);
        assert!(wire.dispatch().is_err());
        let mut byte = 0u8;
        // EOF proves the received duplicate was closed; a leaked writer would
        // make this nonblocking read return EAGAIN instead.
        assert_eq!(
            unsafe { libc::read(reader.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) },
            0
        );
    }
}

//! Private helper process primitive for the glibc payload contract.
//!
//! Preparation and posix_spawn run on a dedicated supervisor, never between a
//! Rust fork and exec. The worker owns the PID until waitpid reaps it. Dropping
//! the handle requests shutdown without blocking the desktop owner. No process
//! group, claimed application PID, shell, PATH search or global environment is
//! involved. This is not yet the managed desktop's Xwayland launcher.
use super::{
    Error, Result,
    payload::PayloadLease,
    readiness::DisplayNotification,
    resources::{DisplayReservation, RuntimeFiles},
};
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr, OsString},
    fs::File,
    io::{self, Read},
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

const GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(10);

/// Parent endpoints for one generation. Insert `wayland` into libwayland on its
/// owner thread and record the actual client identity before spawning `command`.
/// Drive `xwm` with Transport and require both readiness barriers before publish.
pub struct PreparedServer {
    pub command: Command,
    pub environment: (OsString, OsString),
    pub wayland: UnixStream,
    pub xwm: UnixStream,
    pub readiness: DisplayNotification,
}

fn server_arguments(display: u16, authority: &Path, keyboard_data: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        format!(":{display}"),
        "-rootless".into(),
        "-noreset".into(),
        "-nolisten".into(),
        "tcp".into(),
        "-listenfd".into(),
        "6".into(),
        "-listenfd".into(),
        "7".into(),
        "-wm".into(),
        "4".into(),
        "-displayfd".into(),
        "5".into(),
        "-auth".into(),
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend([
        authority.as_os_str().into(),
        "-xkbdir".into(),
        keyboard_data.as_os_str().into(),
        "-fp".into(),
        "built-ins".into(),
    ]);
    args
}

fn server_environment(
    mut environment: BTreeMap<OsString, OsString>,
    payload: &Path,
    scratch: &Path,
) -> Vec<(OsString, OsString)> {
    // Edit only the supplied snapshot. Inherited display/one-shot grants must
    // not redirect the dedicated helper or escape into its compiler child.
    for key in [
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_ACTIVATION_TOKEN",
        "DESKTOP_STARTUP_ID",
        "TELORGON_XKBCOMP_PARENT_PID",
    ] {
        environment.remove(OsStr::new(key));
    }
    for (key, value) in [
        ("WAYLAND_SOCKET", OsString::from("3")),
        (
            "TELORGON_XWAYLAND_PARENT_PID",
            std::process::id().to_string().into(),
        ),
        (
            "TELORGON_XKBCOMP",
            payload.join("bin/xkbcomp").into_os_string(),
        ),
        ("TELORGON_XKB_OUTPUT_DIR", scratch.as_os_str().into()),
    ] {
        environment.insert(key.into(), value);
    }
    environment.into_iter().collect()
}

/// A fully explicit environment and inherited FDs 3 through 7. The caller must
/// supply the dedicated Wayland/XWM sockets, readiness pipe and retained-listener
/// duplicates in that order for Xwayland. FDs 0/1 are null and 2 is drained without
/// recording its contents. Payload leases are retained until the child is reaped.
pub struct Command {
    executable: CString,
    argv: Vec<CString>,
    env: Vec<CString>,
    inherited: Vec<OwnedFd>,
    leases: Vec<Arc<PayloadLease>>,
    resources: Option<(Arc<DisplayReservation>, Arc<RuntimeFiles>)>,
}

impl Command {
    /// Prepare the pinned payload command and fresh private connections, without
    /// executing anything. There is no PATH search or system-server fallback.
    /// The host retains its reservation Arc across generation restarts.
    pub fn embedded(
        payload: Arc<PayloadLease>,
        display: Arc<DisplayReservation>,
        runtime: Arc<RuntimeFiles>,
        environment: BTreeMap<OsString, OsString>,
    ) -> Result<PreparedServer> {
        if display.number() != runtime.display_number() {
            return Err(Error(
                "Xwayland authority and reservation display numbers differ".into(),
            ));
        }
        let (wayland, wayland_child) = UnixStream::pair()?;
        let (xwm, xwm_child) = UnixStream::pair()?;
        let mut pipes = [-1; 2];
        if unsafe { libc::pipe2(pipes.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let reader = unsafe { OwnedFd::from_raw_fd(pipes[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipes[1]) };
        let readiness = DisplayNotification::new(reader, display.number())?;
        let [filesystem, abstract_socket] = display.duplicate_listeners()?;
        let args = server_arguments(
            display.number(),
            &runtime.authority(),
            &payload.path().join("share/X11/xkb"),
        );
        let env = server_environment(environment, payload.path(), &runtime.keymap_directory());
        let mut command = Self::new(
            &payload.path().join("bin/Xwayland"),
            &args,
            &env,
            vec![
                wayland_child.into(),
                xwm_child.into(),
                writer,
                filesystem,
                abstract_socket,
            ],
        )?;
        command.retain_payload(payload);
        let environment = (
            display.display().into(),
            runtime.authority().into_os_string(),
        );
        command.retain_resources(display, runtime)?;
        Ok(PreparedServer {
            command,
            environment,
            wayland,
            xwm,
            readiness,
        })
    }
    pub fn new(
        executable: &Path,
        args: &[OsString],
        environment: &[(OsString, OsString)],
        inherited: Vec<OwnedFd>,
    ) -> Result<Self> {
        if !executable.is_absolute() || inherited.len() > 5 {
            return Err(Error(
                "private helper requires an absolute executable and at most five inherited FDs"
                    .into(),
            ));
        }
        fn string(bytes: &[u8]) -> Result<CString> {
            CString::new(bytes).map_err(|_| Error("private helper argument contains NUL".into()))
        }
        let executable = string(executable.as_os_str().as_bytes())?;
        let mut argv = vec![executable.clone()];
        for arg in args {
            argv.push(string(arg.as_bytes())?);
        }
        let mut env = vec![];
        let mut keys = std::collections::HashSet::new();
        for (key, value) in environment {
            if key.is_empty() || key.as_bytes().contains(&b'=') || !keys.insert(key) {
                return Err(Error(
                    "private helper environment has an invalid or duplicate key".into(),
                ));
            }
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            env.push(string(&entry)?);
        }
        Ok(Self {
            executable,
            argv,
            env,
            inherited,
            leases: vec![],
            resources: None,
        })
    }

    pub fn retain_payload(&mut self, lease: Arc<PayloadLease>) {
        self.leases.push(lease);
    }

    /// Hold the listener reservation and authentication/scratch files until this
    /// child is reaped. The host should keep its own reservation Arc for retries.
    pub fn retain_resources(
        &mut self,
        display: Arc<DisplayReservation>,
        runtime: Arc<RuntimeFiles>,
    ) -> Result<()> {
        if display.number() != runtime.display_number() {
            return Err(Error(
                "Xwayland authority and reservation display numbers differ".into(),
            ));
        }
        self.resources = Some((display, runtime));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    /// Stopped before process creation.
    Cancelled,
    Running {
        pid: i32,
    },
    /// Raw wait status, decoded without treating a signal as a successful exit.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// Includes spawn/wait errors but never helper output or environment values.
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    /// Content-free accounting; diagnostic bytes are discarded as they arrive.
    pub diagnostic_bytes: u64,
}

pub struct Supervisor {
    stop: SyncSender<()>,
    wake: UnixStream,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl Supervisor {
    /// Returns before process creation. Register fd() for readable readiness,
    /// then call snapshot(); notifications coalesce and the snapshot is durable.
    pub fn spawn(command: Command) -> Result<Self> {
        let (wake, mut notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        notify.set_nonblocking(true)?;
        let (stop, receiver) = mpsc::sync_channel(1);
        let snapshot = Arc::new(Mutex::new(Snapshot {
            status: Status::Starting,
            diagnostic_bytes: 0,
        }));
        let shared = snapshot.clone();
        std::thread::Builder::new()
            .name("telorgon-xwayland".into())
            .spawn(move || {
                let result = supervise(command, receiver, &shared, &mut notify);
                if let Err(error) = result {
                    shared.lock().unwrap().status = Status::Failed(error.to_string());
                }
                // MSG_NOSIGNAL prevents a dropped host handle from signalling the
                // compositor. EAGAIN is harmless: an earlier notification is pending.
                notify_change(&notify);
            })?;
        Ok(Self {
            stop,
            wake,
            snapshot,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub fn snapshot(&mut self) -> Snapshot {
        let mut bytes = [0u8; 64];
        loop {
            match self.wake.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.snapshot.lock().unwrap().clone()
    }

    /// Idempotent. The worker sends TERM once, then KILL after two seconds.
    pub fn stop(&self) {
        let _ = self.stop.try_send(());
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn notify_change(socket: &UnixStream) {
    unsafe {
        libc::send(
            socket.as_raw_fd(),
            [1u8].as_ptr().cast(),
            1,
            libc::MSG_NOSIGNAL,
        );
    }
}

fn checked(code: i32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code).into())
    }
}

struct Actions(libc::posix_spawn_file_actions_t);
impl Drop for Actions {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.0);
        }
    }
}
struct Attributes(libc::posix_spawnattr_t);
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.0);
        }
    }
}

fn spawn(command: &Command, diagnostic: OwnedFd) -> Result<i32> {
    let null = File::options().read(true).write(true).open("/dev/null")?;
    let sources = [null.as_raw_fd(), null.as_raw_fd(), diagnostic.as_raw_fd()];
    // Relocate every source above every destination before adding any dup2.
    // This also handles the caller having closed standard FDs before preparation.
    let mut relocated = vec![];
    for fd in sources
        .into_iter()
        .chain(command.inherited.iter().map(AsRawFd::as_raw_fd))
    {
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 8) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error().into());
        }
        relocated.push(unsafe { OwnedFd::from_raw_fd(duplicate) });
    }
    let mut actions = MaybeUninit::uninit();
    checked(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
    let mut actions = Actions(unsafe { actions.assume_init() });
    for (destination, source) in relocated.iter().enumerate() {
        checked(unsafe {
            libc::posix_spawn_file_actions_adddup2(
                &mut actions.0,
                source.as_raw_fd(),
                destination as i32,
            )
        })?;
    }
    checked(unsafe {
        libc::posix_spawn_file_actions_addclosefrom_np(&mut actions.0, relocated.len() as i32)
    })?;
    let mut attr = MaybeUninit::uninit();
    checked(unsafe { libc::posix_spawnattr_init(attr.as_mut_ptr()) })?;
    let mut attr = Attributes(unsafe { attr.assume_init() });
    let mut empty = MaybeUninit::uninit();
    let mut defaults = MaybeUninit::uninit();
    unsafe {
        libc::sigemptyset(empty.as_mut_ptr());
        libc::sigfillset(defaults.as_mut_ptr());
    }
    let empty = unsafe { empty.assume_init() };
    let defaults = unsafe { defaults.assume_init() };
    checked(unsafe { libc::posix_spawnattr_setsigmask(&mut attr.0, &empty) })?;
    checked(unsafe { libc::posix_spawnattr_setsigdefault(&mut attr.0, &defaults) })?;
    checked(unsafe {
        libc::posix_spawnattr_setflags(
            &mut attr.0,
            (libc::POSIX_SPAWN_SETSIGMASK | libc::POSIX_SPAWN_SETSIGDEF) as i16,
        )
    })?;
    let argv: Vec<_> = command
        .argv
        .iter()
        .map(|v| v.as_ptr().cast_mut())
        .chain([std::ptr::null_mut()])
        .collect();
    let env: Vec<_> = command
        .env
        .iter()
        .map(|v| v.as_ptr().cast_mut())
        .chain([std::ptr::null_mut()])
        .collect();
    let mut pid = 0;
    checked(unsafe {
        libc::posix_spawn(
            &mut pid,
            command.executable.as_ptr(),
            &actions.0,
            &attr.0,
            argv.as_ptr(),
            env.as_ptr(),
        )
    })?;
    Ok(pid)
}

fn supervise(
    command: Command,
    stop: Receiver<()>,
    snapshot: &Mutex<Snapshot>,
    notify: &mut UnixStream,
) -> Result<()> {
    let mut pipes = [-1; 2];
    if unsafe { libc::pipe2(pipes.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut diagnostics = unsafe { File::from_raw_fd(pipes[0]) };
    let writer = unsafe { OwnedFd::from_raw_fd(pipes[1]) };
    if unsafe { libc::fcntl(diagnostics.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // A cancelled preparation never creates a process.
    if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
        snapshot.lock().unwrap().status = Status::Cancelled;
        return Ok(());
    }
    let pid = spawn(&command, writer)?;
    // Close parent copies of child endpoints immediately, but retain the payload.
    let _leases = command.leases;
    let _resources = command.resources;
    drop(command.inherited);
    snapshot.lock().unwrap().status = Status::Running { pid };
    notify_change(notify);
    let mut deadline = None;
    let mut killed = false;
    loop {
        // Bound each diagnostic drain. Output cannot grow host memory, and
        // continuous logging cannot starve cancellation or waitpid.
        let mut count = 0u64;
        let mut buffer = [0u8; 8192];
        for _ in 0..32 {
            match diagnostics.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => count += n as u64,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        {
            let mut snapshot = snapshot.lock().unwrap();
            snapshot.diagnostic_bytes = snapshot.diagnostic_bytes.saturating_add(count);
        }
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if waited == pid {
            snapshot.lock().unwrap().status = Status::Exited {
                code: libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status)),
                signal: libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status)),
            };
            return Ok(());
        }
        if waited < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // ECHILD means somebody else reaped it; never signal a reused PID.
            return Err(error.into());
        }
        if deadline.is_some_and(|time| Instant::now() >= time) && !killed {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            killed = true;
        }
        if deadline.is_none() {
            match stop.recv_timeout(POLL) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    deadline = Some(Instant::now() + GRACE);
                }
            }
        } else {
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_arguments_and_environment_preserve_literal_private_paths() {
        let args = server_arguments(
            42,
            Path::new("/private space/authority"),
            Path::new("/payload;literal/share/X11/xkb"),
        );
        let expected: Vec<OsString> = [
            ":42",
            "-rootless",
            "-noreset",
            "-nolisten",
            "tcp",
            "-listenfd",
            "6",
            "-listenfd",
            "7",
            "-wm",
            "4",
            "-displayfd",
            "5",
            "-auth",
            "/private space/authority",
            "-xkbdir",
            "/payload;literal/share/X11/xkb",
            "-fp",
            "built-ins",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        assert_eq!(args, expected);
        let input: BTreeMap<OsString, OsString> = [
            ("DISPLAY", ":old"),
            ("XAUTHORITY", "/old"),
            ("WAYLAND_DISPLAY", "old"),
            ("WAYLAND_SOCKET", "999"),
            ("TELORGON_XKBCOMP", "/old"),
            ("TELORGON_XWAYLAND_PARENT_PID", "1"),
            ("XDG_ACTIVATION_TOKEN", "old"),
            ("DESKTOP_STARTUP_ID", "old"),
            ("TELORGON_XKBCOMP_PARENT_PID", "1"),
            ("LANG", "de_DE.UTF-8"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let env: BTreeMap<_, _> = server_environment(
            input.clone(),
            Path::new("/payload;literal"),
            Path::new("/private space/keymaps"),
        )
        .into_iter()
        .collect();
        for key in [
            "DISPLAY",
            "XAUTHORITY",
            "WAYLAND_DISPLAY",
            "XDG_ACTIVATION_TOKEN",
            "DESKTOP_STARTUP_ID",
            "TELORGON_XKBCOMP_PARENT_PID",
        ] {
            assert!(!env.contains_key(OsStr::new(key)));
        }
        assert_eq!(env[OsStr::new("WAYLAND_SOCKET")], "3");
        assert_eq!(
            env[OsStr::new("TELORGON_XKBCOMP")],
            "/payload;literal/bin/xkbcomp"
        );
        assert_eq!(
            env[OsStr::new("TELORGON_XKB_OUTPUT_DIR")],
            "/private space/keymaps"
        );
        assert_eq!(
            env[OsStr::new("TELORGON_XWAYLAND_PARENT_PID")],
            std::process::id().to_string().as_str()
        );
        assert_eq!(env[OsStr::new("LANG")], "de_DE.UTF-8");
        assert_eq!(input[OsStr::new("WAYLAND_SOCKET")], "999");
    }
    fn python(code: &str, inherited: Vec<OwnedFd>) -> Command {
        Command::new(
            Path::new("/usr/bin/python3"),
            &["-c".into(), code.into()],
            &[],
            inherited,
        )
        .unwrap()
    }
    fn finish(child: &mut Supervisor) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let snapshot = child.snapshot();
            if matches!(
                snapshot.status,
                Status::Exited { .. } | Status::Failed(_) | Status::Cancelled
            ) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "fixture failed to exit");
            std::thread::sleep(POLL);
        }
    }
    #[test]
    fn literal_arguments_explicit_environment_and_fd_contract() {
        let null = File::open("/dev/null").unwrap();
        // Deliberately non-CLOEXEC: closefrom must remove unrelated descriptors.
        let raw = unsafe { libc::fcntl(null.as_raw_fd(), libc::F_DUPFD, 100) };
        assert!(raw >= 100);
        let _unrelated = unsafe { OwnedFd::from_raw_fd(raw) };
        let (mut read, write) = UnixStream::pair().unwrap();
        read.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let command = Command::new(Path::new("/usr/bin/python3"), &[
            "-c".into(),
            "import os,sys; assert sys.argv[1] == '$(touch forbidden); space'; assert os.environ.get('TELOGRON_TEST') == 'literal'; assert 'HOME' not in os.environ; assert os.read(0,1) == b''; assert not [f for f in range(4,256) if os.path.exists('/proc/self/fd/'+str(f))]; os.write(3,b'passed')".into(),
            "$(touch forbidden); space".into(),
        ], &[("TELOGRON_TEST".into(), "literal".into())], vec![write.into()]).unwrap();
        let mut child = Supervisor::spawn(command).unwrap();
        let result = finish(&mut child);
        assert_eq!(
            result.status,
            Status::Exited {
                code: Some(0),
                signal: None
            }
        );
        let mut bytes = vec![];
        read.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"passed");
    }
    #[test]
    fn drains_large_diagnostics_without_retaining_content() {
        let mut child =
            Supervisor::spawn(python("import os; os.write(2, b'x' * (1024*1024))", vec![]))
                .unwrap();
        let result = finish(&mut child);
        assert_eq!(
            result.status,
            Status::Exited {
                code: Some(0),
                signal: None
            }
        );
        assert_eq!(result.diagnostic_bytes, 1024 * 1024);
    }
    #[test]
    fn ignored_term_is_killed_and_reaped_off_owner_thread() {
        let (mut read, write) = UnixStream::pair().unwrap();
        read.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut child = Supervisor::spawn(python("import os,signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); os.write(3,b'r'); time.sleep(30)", vec![write.into()])).unwrap();
        let mut ready = [0];
        read.read_exact(&mut ready).unwrap();
        let pid = match child.snapshot().status {
            Status::Running { pid } => pid,
            state => panic!("{state:?}"),
        };
        let start = Instant::now();
        child.stop();
        assert!(start.elapsed() < Duration::from_millis(100));
        let result = finish(&mut child);
        assert_eq!(
            result.status,
            Status::Exited {
                code: None,
                signal: Some(libc::SIGKILL)
            }
        );
        assert!(start.elapsed() >= GRACE);
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    #[test]
    fn failed_exec_and_invalid_command_are_contained() {
        assert!(Command::new(Path::new("Xwayland"), &[], &[], vec![]).is_err());
        assert!(
            Command::new(
                Path::new("/missing"),
                &[],
                &[("A".into(), "1".into()), ("A".into(), "2".into())],
                vec![]
            )
            .is_err()
        );
        let mut child = Supervisor::spawn(
            Command::new(Path::new("/nonexistent/telorgon-helper"), &[], &[], vec![]).unwrap(),
        )
        .unwrap();
        assert!(matches!(finish(&mut child).status, Status::Failed(_)));
    }

    #[test]
    fn all_five_inherited_descriptors_reach_their_exact_destinations() {
        let mut readers = vec![];
        let mut writers = vec![];
        for _ in 3..8 {
            let (read, write) = UnixStream::pair().unwrap();
            read.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            readers.push(read);
            writers.push(write.into());
        }
        let mut child = Supervisor::spawn(python("import os; [os.write(fd,bytes([fd])) for fd in range(3,8)]; assert not [fd for fd in range(8,256) if os.path.exists('/proc/self/fd/'+str(fd))]", writers)).unwrap();
        assert_eq!(
            finish(&mut child).status,
            Status::Exited {
                code: Some(0),
                signal: None
            }
        );
        for (index, read) in readers.iter_mut().enumerate() {
            let mut bytes = vec![];
            read.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, vec![(index + 3) as u8]);
        }
    }

    #[test]
    fn dropped_handle_still_terminates_and_reaps_its_child() {
        let (mut read, write) = UnixStream::pair().unwrap();
        read.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut child = Supervisor::spawn(python(
            "import os,time; os.write(3,b'r'); time.sleep(30)",
            vec![write.into()],
        ))
        .unwrap();
        read.read_exact(&mut [0]).unwrap();
        let pid = match child.snapshot().status {
            Status::Running { pid } => pid,
            state => panic!("{state:?}"),
        };
        let snapshot = child.snapshot.clone();
        let start = Instant::now();
        drop(child);
        assert!(start.elapsed() < Duration::from_millis(100));
        while !matches!(snapshot.lock().unwrap().status, Status::Exited { .. }) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(POLL);
        }
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}

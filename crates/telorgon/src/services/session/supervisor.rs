use super::command::{Command, Completion, ManagedChild, ProcessOutput, RestartPolicy, Stream};
use super::recovery::{Journal, RecoveryEntry, process_identity, restored_command};
use super::{Environment, Error, Result, SESSION, SessionConfig};
use async_process::{Child, Command as NativeCommand, Stdio};
use futures_lite::io::{AsyncRead, AsyncWrite};
use std::ffi::OsStr;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Ready,
    Quiescing,
    Closing,
    Closed,
}

/// Thread-safe launch capability; contains no graphics, input or libseat objects.
#[derive(Clone)]
pub struct SessionHandle {
    worker: Arc<Worker>,
}

struct Worker {
    state: Mutex<State>,
    thread: OnceLock<Thread>,
    env: Environment,
    config: SessionConfig,
}

struct State {
    x11_startup_pending: bool,
    deferred: Vec<DeferredLaunch>,
    x11_environment: Option<(std::ffi::OsString, std::ffi::OsString)>,
    phase: SessionPhase,
    children: Vec<Running>,
    pending: Vec<RecoveryEntry>,
    journal: Option<Journal>,
    dirty: bool,
    next_id: u64,
    closing_at: Option<Instant>,
    aborting: bool,
    errors: Vec<Error>,
}

struct DeferredLaunch {
    command: Command,
    completion: Arc<Completion>,
    entry: RecoveryEntry,
}
impl State {
    fn launch_environment(&self, base: &Environment) -> Environment {
        let mut environment = base.clone();
        if let Some((display, authority)) = &self.x11_environment {
            environment.0.insert("DISPLAY".into(), display.clone());
            environment.0.insert("XAUTHORITY".into(), authority.clone());
        }
        environment
    }
    #[cfg(feature = "shell-xwayland")]
    fn set_x11_environment(
        &mut self,
        environment: Option<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> bool {
        if self.x11_environment == environment && !self.x11_startup_pending {
            return false;
        }
        self.x11_startup_pending = false;
        if self.x11_environment.is_some() {
            // Conservative incident scope: managed children that inherited this
            // display, regardless of toolkit or untrusted _NET_WM_PID metadata.
            for child in &mut self.children {
                if child.inherited_x11 {
                    child.retry_suppressed = true;
                    child.restart_at = None;
                }
            }
        }
        self.x11_environment = environment;
        true
    }
}

struct Running {
    inherited_x11: bool,
    retry_suppressed: bool,
    child: Child,
    command: Command,
    completion: Arc<Completion>,
    entry: RecoveryEntry,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    status: Option<ExitStatus>,
    exited_at: Option<Instant>,
    restart_at: Option<Instant>,
    retries: u8,
    terminate_sent: bool,
    capture_error: Option<Error>,
    input_position: usize,
    #[cfg(target_os = "linux")]
    pidfd: Option<std::os::fd::OwnedFd>,
}

impl Worker {
    fn notify(&self) {
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }
}
struct WorkerWake(Weak<Worker>);
impl Wake for WorkerWake {
    fn wake(self: Arc<Self>) {
        if let Some(worker) = self.0.upgrade() {
            worker.notify();
        }
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(worker) = self.0.upgrade() {
            worker.notify();
        }
    }
}

impl SessionHandle {
    pub fn phase(&self) -> SessionPhase {
        self.worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .phase
    }
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command.session = Some(self.clone());
        command
    }
    pub fn application(&self, id: impl Into<String>) -> super::ApplicationRequest {
        let mut request = super::ApplicationRequest::new(id.into());
        request.session = Some(self.clone());
        request
    }
    /// Snapshot of the environment supplied to children. Does not expose the global mutable env.
    pub fn environment(&self, key: &str) -> Option<std::ffi::OsString> {
        let state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .launch_environment(&self.worker.env)
            .get(key)
            .map(Into::into)
    }
    pub fn pending_recovery(&self) -> Result<Vec<RecoveryEntry>> {
        let state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.phase != SessionPhase::Ready {
            return Err(Error::Closing);
        }
        Ok(state.pending.clone())
    }
    pub fn dismiss_recovery(&self, id: u64) -> Result<()> {
        let mut state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.phase != SessionPhase::Ready {
            return Err(Error::Closing);
        }
        state.pending.retain(|entry| entry.id != id);
        state.dirty = true;
        self.worker.notify();
        Ok(())
    }
    pub fn restore(&self, id: u64) -> Result<ManagedChild> {
        let mut state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .pending
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| Error::Invalid("unknown recovery entry".into()))?;
        if entry.original_process_alive() {
            return Err(Error::Invalid("the previous process is still alive; close it before restoring to avoid duplicate applications".into()));
        }
        state.pending.retain(|entry| entry.id != id);
        match self.spawn_locked(restored_command(&entry, self), &mut state) {
            Ok(child) => Ok(child),
            Err(error) => {
                state.pending.push(entry);
                Err(error)
            }
        }
    }
    pub(crate) fn restore_entry(&self, entry: &RecoveryEntry) -> Result<ManagedChild> {
        let mut state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        let saved = state
            .pending
            .iter()
            .find(|saved| saved.id == entry.id && saved.generation == entry.generation)
            .cloned()
            .ok_or_else(|| Error::Invalid("stale recovery entry".into()))?;
        if saved.original_process_alive() {
            return Err(Error::Invalid(
                "the original application is still running".into(),
            ));
        }
        state.pending.retain(|entry| entry.id != saved.id);
        match self.spawn_locked(restored_command(&saved, self), &mut state) {
            Ok(child) => Ok(child),
            Err(error) => {
                state.pending.push(saved);
                Err(error)
            }
        }
    }
    /// Diagnostics include failed crash retries, recovery writes, and children that refused exit.
    pub fn take_diagnostics(&self) -> Vec<Error> {
        std::mem::take(
            &mut self
                .worker
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .errors,
        )
    }
    pub(crate) fn config(&self) -> &SessionConfig {
        &self.worker.config
    }
    pub(crate) fn env(&self) -> &Environment {
        &self.worker.env
    }

    pub(crate) fn spawn(&self, command: Command) -> Result<ManagedChild> {
        let mut state = self.worker.state.lock().unwrap_or_else(|e| e.into_inner());
        self.spawn_locked(command, &mut state)
    }
    fn spawn_locked(&self, mut command: Command, state: &mut State) -> Result<ManagedChild> {
        if state.phase != SessionPhase::Ready {
            return Err(Error::Closing);
        }
        // Recovery offers are metadata, not running or queued processes.
        if state.children.len() + state.deferred.len() >= 256 {
            return Err(Error::ProcessLimit);
        }
        if command.spec.program.is_empty()
            || command.stdin == Stream::Capture
            || command.limit == 0
            || command.limit > 64 * 1024 * 1024
            || command
                .input
                .as_ref()
                .is_some_and(|bytes| bytes.len() > 8 * 1024 * 1024)
        {
            return Err(Error::Invalid(
                "invalid program, stdin capture, output limit (1 byte..64 MiB), or input size (maximum 8 MiB)".into(),
            ));
        }
        if command.spec.recover
            && (!command.env.is_empty()
                || command.input.is_some()
                || command.stdin != Stream::Inherit
                || command.stdout != Stream::Inherit
                || command.stderr != Stream::Inherit)
        {
            return Err(Error::Invalid("persistent recovery requires inherited streams, no piped input, and no environment overrides; use recover(false) for customized commands".into()));
        }
        // Capture cwd once, before execution, so later retries never depend on another thread's cwd.
        let base = std::env::current_dir()?;
        command.spec.directory = Some(match &command.spec.directory {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => base.join(path),
            None => base,
        });
        // Avoid a reference cycle: stored commands use this worker's environment directly.
        command.session = None;
        let id = state.next_id;
        let next_id = state.next_id.checked_add(1).ok_or(Error::ProcessLimit)?;
        if state.x11_startup_pending {
            let completion = Arc::new(Completion::new(0));
            let entry = RecoveryEntry {
                id,
                spec: command.spec.clone(),
                pid: 0,
                process_identity: None,
                generation: id,
            };
            state.deferred.push(DeferredLaunch {
                command,
                completion: completion.clone(),
                entry,
            });
            state.next_id = next_id;
            state.dirty = true;
            self.worker.notify();
            return Ok(ManagedChild { completion });
        }
        let inherited_x11 = state.x11_environment.as_ref().is_some_and(|(display, _)| {
            command
                .env
                .get(OsStr::new("DISPLAY"))
                .map_or(true, |value| value.as_ref() == Some(display))
        });
        let child = spawn_native(&command, &state.launch_environment(&self.worker.env))?;
        let completion = Arc::new(Completion::new(child.id()));
        state.next_id = next_id;
        state.children.push(Running::new(
            child,
            command,
            completion.clone(),
            id,
            inherited_x11,
        ));
        state.dirty = true;
        self.worker.notify();
        Ok(ManagedChild { completion })
    }
}

fn spawn_native(command: &Command, env: &Environment) -> Result<Child> {
    let mut native = NativeCommand::new(&command.spec.program);
    native.args(&command.spec.args).env_clear().envs(&env.0);
    for (key, value) in &command.env {
        match value {
            Some(value) => {
                native.env(key, value);
            }
            None => {
                native.env_remove(key);
            }
        }
    }
    if let Some(path) = &command.spec.directory {
        native.current_dir(path);
    }
    if let Some(token) = &command.activation_token {
        native.env("XDG_ACTIVATION_TOKEN", token);
    }
    let stream = |mode| match mode {
        Stream::Inherit => Stdio::inherit(),
        Stream::Null => Stdio::null(),
        Stream::Capture => Stdio::piped(),
    };
    native
        .stdin(if command.input.is_some() {
            Stdio::piped()
        } else {
            stream(command.stdin)
        })
        .stdout(stream(command.stdout))
        .stderr(stream(command.stderr));
    // Reap on drop without implicitly killing; graceful shutdown belongs to the host.
    native.reap_on_drop(true).kill_on_drop(false);
    native
        .spawn()
        .map_err(|e| Error::Io(format!("could not launch {:?}: {e}", command.spec.program)))
}

impl Running {
    fn new(
        mut child: Child,
        mut command: Command,
        completion: Arc<Completion>,
        id: u64,
        inherited_x11: bool,
    ) -> Self {
        command.activation_token = None; // Single-use; never persist or replay on retry.
        let entry = RecoveryEntry {
            id,
            spec: command.spec.clone(),
            pid: child.id(),
            process_identity: process_identity(child.id()),
            generation: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
        };
        #[cfg(target_os = "linux")]
        let pidfd = {
            use std::os::fd::FromRawFd;
            // A pidfd cannot be redirected to a recycled PID. Check child status after opening it;
            // async-process may already have reaped a very short-lived process.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0) };
            let fd = if fd >= 0 {
                Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) })
            } else {
                None
            };
            if matches!(child.try_status(), Ok(None)) {
                fd
            } else {
                None
            }
        };
        Self {
            inherited_x11,
            retry_suppressed: false,
            child,
            command,
            completion,
            entry,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            status: None,
            exited_at: None,
            restart_at: None,
            retries: 0,
            terminate_sent: false,
            capture_error: None,
            input_position: 0,
            #[cfg(target_os = "linux")]
            pidfd,
        }
    }

    fn terminate(&mut self) -> Result<()> {
        if self.terminate_sent || self.child.try_status()?.is_some() {
            return Ok(());
        }
        self.terminate_sent = true;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let fd = self.pidfd.as_ref().ok_or_else(|| Error::Unsupported(
                "graceful process termination requires Linux pidfd support; no unsafe PID-based fallback is used".into()))?;
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    libc::SIGTERM,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            };
            if result < 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    return Err(e.into());
                }
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(Error::Unsupported(
            "automatic graceful child termination is currently implemented on Linux only".into(),
        ))
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        phase: SessionPhase,
        env: &Environment,
    ) -> Result<bool> {
        if let Some(at) = self.restart_at {
            if phase != SessionPhase::Ready {
                return Ok(true);
            }
            if Instant::now() < at {
                return Ok(false);
            }
            let retries = self.retries;
            let child = spawn_native(&self.command, env)?;
            self.completion.pid.store(child.id(), Ordering::Release);
            *self = Self::new(
                child,
                self.command.clone(),
                self.completion.clone(),
                self.entry.id,
                self.inherited_x11,
            );
            self.retries = retries;
        }
        self.write_input(cx);
        if let Err(error) = drain(
            &mut self.child.stdout,
            &mut self.stdout,
            &mut self.stdout_truncated,
            self.command.limit,
            cx,
        ) {
            self.child.stdout = None;
            self.capture_error = Some(error);
        }
        if let Err(error) = drain(
            &mut self.child.stderr,
            &mut self.stderr,
            &mut self.stderr_truncated,
            self.command.limit,
            cx,
        ) {
            self.child.stderr = None;
            self.capture_error = Some(error);
        }
        if self.status.is_none() {
            self.status = self.child.try_status()?;
            if self.status.is_some() {
                self.exited_at = Some(Instant::now());
            }
        }
        let Some(status) = self.status else {
            return Ok(false);
        };
        // Descendants can inherit a pipe. Never wait forever after the direct child has exited.
        if self.child.stdout.is_some() || self.child.stderr.is_some() {
            if self.exited_at.unwrap().elapsed() < Duration::from_millis(250) {
                return Ok(false);
            }
            self.stdout_truncated |= self.child.stdout.take().is_some();
            self.stderr_truncated |= self.child.stderr.take().is_some();
        }
        if phase == SessionPhase::Ready
            && !self.retry_suppressed
            && !status.success()
            && self.command.spec.restart == RestartPolicy::OnFailure
            && self.retries < 3
        {
            self.restart_at = Some(Instant::now() + Duration::from_secs(1 << self.retries));
            self.retries += 1;
            return Ok(false);
        }
        Ok(true)
    }
    fn output(&mut self) -> Result<ProcessOutput> {
        if let Some(error) = self.capture_error.take() {
            return Err(error);
        }
        Ok(ProcessOutput {
            status: self.status.ok_or(Error::Closing)?,
            stdout: std::mem::take(&mut self.stdout),
            stderr: std::mem::take(&mut self.stderr),
            stdout_truncated: self.stdout_truncated,
            stderr_truncated: self.stderr_truncated,
        })
    }
    fn write_input(&mut self, cx: &mut Context<'_>) {
        let Some(input) = &self.command.input else {
            return;
        };
        let Some(pipe) = self.child.stdin.as_mut() else {
            return;
        };
        for _ in 0..8 {
            if self.input_position == input.len() {
                self.child.stdin = None;
                return;
            }
            let end = (self.input_position + 8192).min(input.len());
            match Pin::new(&mut *pipe).poll_write(cx, &input[self.input_position..end]) {
                Poll::Ready(Ok(0)) => {
                    self.child.stdin = None;
                    return;
                }
                Poll::Ready(Ok(count)) => self.input_position += count,
                Poll::Ready(Err(error)) => {
                    if error.kind() != std::io::ErrorKind::BrokenPipe {
                        self.capture_error = Some(error.into());
                    }
                    self.child.stdin = None;
                    return;
                }
                Poll::Pending => return,
            }
        }
        cx.waker().wake_by_ref();
    }
}

fn drain<R: AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    bytes: &mut Vec<u8>,
    truncated: &mut bool,
    limit: usize,
    cx: &mut Context<'_>,
) -> Result<()> {
    let Some(reader) = pipe.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0; 8192];
    // Fairness budget: one noisy child cannot starve other children or shutdown.
    for _ in 0..8 {
        match Pin::new(&mut *reader).poll_read(cx, &mut buffer) {
            Poll::Ready(Ok(0)) => {
                *pipe = None;
                return Ok(());
            }
            Poll::Ready(Ok(count)) => {
                let keep = count.min(limit.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&buffer[..keep]);
                *truncated |= keep < count;
            }
            Poll::Ready(Err(e)) => return Err(e.into()),
            Poll::Pending => return Ok(()),
        }
    }
    cx.waker().wake_by_ref();
    Ok(())
}

pub(crate) struct SessionOwner {
    handle: SessionHandle,
    thread: Option<JoinHandle<()>>,
    #[cfg(target_os = "linux")]
    services: Option<super::bus::ServiceEnvironment>,
}

impl SessionOwner {
    pub(crate) fn start(env: Environment, config: SessionConfig) -> Result<Self> {
        Self::start_impl(env, config, false, false)
    }
    pub(crate) fn start_gui(env: Environment, config: SessionConfig) -> Result<Self> {
        Self::start_impl(env, config, true, false)
    }
    #[cfg(feature = "shell-xwayland")]
    #[cfg_attr(not(feature = "shell-xwayland-embedded"), allow(dead_code))]
    pub(crate) fn start_waiting_for_x11(env: Environment, config: SessionConfig) -> Result<Self> {
        Self::start_impl(env, config, false, true)
    }
    fn start_impl(
        env: Environment,
        config: SessionConfig,
        gui: bool,
        x11_startup_pending: bool,
    ) -> Result<Self> {
        config.validate()?;
        let mut registry = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        if registry.is_some() {
            return Err(Error::AlreadyRunning);
        }
        let mut errors = Vec::new();
        let (journal, pending) = match Journal::open(&config, &env) {
            Ok(Some((j, p))) => (Some(j), p),
            Ok(None) => (None, Vec::new()),
            Err(Error::RecoveryInUse) if gui => {
                // A second instance of an ordinary app must still open. It does not steal or
                // overwrite the first instance's recovery journal.
                errors.push(Error::RecoveryInUse);
                (None, Vec::new())
            }
            Err(error) => return Err(error),
        };
        let next_id = pending
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(Error::ProcessLimit)?;
        let worker = Arc::new(Worker {
            state: Mutex::new(State {
                x11_startup_pending,
                deferred: Vec::new(),
                x11_environment: None,
                phase: SessionPhase::Ready,
                children: Vec::new(),
                pending,
                journal,
                dirty: false,
                next_id,
                closing_at: None,
                aborting: false,
                errors,
            }),
            thread: OnceLock::new(),
            env,
            config,
        });
        let handle = SessionHandle {
            worker: worker.clone(),
        };
        let thread = thread::Builder::new()
            .name("telorgon-processes".into())
            .spawn(move || run_worker(worker))?;
        *registry = Some(handle.clone());
        Ok(Self {
            handle,
            thread: Some(thread),
            #[cfg(target_os = "linux")]
            services: None,
        })
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn publish_services(&mut self) -> Result<()> {
        self.services = Some(super::bus::ServiceEnvironment::publish(
            &self.handle.worker.env,
        )?);
        Ok(())
    }
    #[cfg(feature = "shell-xwayland")]
    pub(crate) fn set_x11_environment(
        &self,
        environment: Option<(std::ffi::OsString, std::ffi::OsString)>,
    ) {
        let changed = self
            .handle
            .worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_x11_environment(environment);
        if changed {
            self.handle.worker.notify();
        }
    }
    pub(crate) fn quiesce(&self) {
        self.handle
            .worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .phase = SessionPhase::Quiescing;
        self.handle.worker.notify();
    }
    pub(crate) fn resume(&self) {
        self.handle
            .worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .phase = SessionPhase::Ready;
        self.handle.worker.notify();
    }
    pub(crate) fn close(&self) {
        let mut state = self
            .handle
            .worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.phase == SessionPhase::Closed {
            return;
        }
        state.phase = SessionPhase::Closing;
        state.closing_at.get_or_insert_with(Instant::now);
        self.handle.worker.notify();
    }
    /// Startup/runtime failure preserves active recoverable launches instead of treating it as
    /// successful logout. In particular, unwinding must not erase recovery state.
    pub(crate) fn abort(&self) {
        let mut state = self
            .handle
            .worker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.phase == SessionPhase::Closed {
            return;
        }
        state.aborting = true;
        state.phase = SessionPhase::Closing;
        self.handle.worker.notify();
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) {
        if self.handle.phase() != SessionPhase::Closing
            && self.handle.phase() != SessionPhase::Closed
        {
            self.abort();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let mut registry = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .as_ref()
            .is_some_and(|h| Arc::ptr_eq(&h.worker, &self.handle.worker))
        {
            *registry = None;
        }
        for error in self.handle.take_diagnostics() {
            eprintln!("telorgon-session: {error}");
        }
    }
}

fn run_worker(worker: Arc<Worker>) {
    let _ = worker.thread.set(thread::current());
    let waker = Waker::from(Arc::new(WorkerWake(Arc::downgrade(&worker))));
    loop {
        let mut completed = Vec::new();
        let (closed, idle) = {
            let mut state = worker.state.lock().unwrap_or_else(|e| e.into_inner());
            let phase = state.phase;
            let expired = state.aborting
                || state
                    .closing_at
                    .is_some_and(|at| at.elapsed() >= worker.config.shutdown_timeout);
            let environment = state.launch_environment(&worker.env);
            if phase == SessionPhase::Closing
                || (phase == SessionPhase::Ready && !state.x11_startup_pending)
            {
                let count = if phase == SessionPhase::Closing {
                    state.deferred.len()
                } else {
                    state.deferred.len().min(4)
                };
                let queued: Vec<_> = state.deferred.drain(..count).collect();
                for launch in queued {
                    let inherited_x11 =
                        state.x11_environment.as_ref().is_some_and(|(display, _)| {
                            launch
                                .command
                                .env
                                .get(OsStr::new("DISPLAY"))
                                .map_or(true, |value| value.as_ref() == Some(display))
                        });
                    let result = if phase == SessionPhase::Closing {
                        Err(Error::Closing)
                    } else {
                        spawn_native(&launch.command, &environment)
                    };
                    match result {
                        Ok(child) => {
                            launch.completion.pid.store(child.id(), Ordering::Release);
                            state.children.push(Running::new(
                                child,
                                launch.command,
                                launch.completion,
                                launch.entry.id,
                                inherited_x11,
                            ));
                        }
                        Err(error) => {
                            if phase != SessionPhase::Closing {
                                record_error(&mut state, error.clone());
                            }
                            if launch.command.spec.recover {
                                state.pending.push(launch.entry);
                            }
                            completed.push((launch.completion, Err(error)));
                        }
                    }
                    state.dirty = true;
                }
            }
            let mut index = 0;
            while index < state.children.len() {
                if phase == SessionPhase::Closing && !state.aborting {
                    if let Err(e) = state.children[index].terminate() {
                        record_error(&mut state, e);
                    }
                }
                let old_pid = state.children[index].child.id();
                let result = state.children[index].poll(
                    &mut Context::from_waker(&waker),
                    phase,
                    &environment,
                );
                if old_pid != state.children[index].child.id() {
                    state.dirty = true;
                }
                if expired || !matches!(result, Ok(false)) {
                    let mut child = state.children.remove(index);
                    let output = match result {
                        Err(e) => Err(e),
                        _ if expired && child.status.is_none() => Err(Error::Closing),
                        _ => child.output(),
                    };
                    // Failed or still-running recoverable applications remain offered for recovery.
                    if child.command.spec.recover
                        && (output.is_err()
                            || (phase == SessionPhase::Ready
                                && output.as_ref().is_ok_and(|o| !o.status.success())))
                    {
                        state.pending.push(child.entry.clone());
                    }
                    if let Err(e) = &output {
                        record_error(&mut state, e.clone());
                    }
                    completed.push((child.completion.clone(), output));
                    state.dirty = true;
                } else {
                    index += 1;
                }
            }
            if state.dirty {
                if let Some(journal) = &state.journal {
                    let entries = state
                        .pending
                        .iter()
                        .cloned()
                        .chain(
                            state
                                .children
                                .iter()
                                .filter(|child| child.command.spec.recover)
                                .map(|child| child.entry.clone()),
                        )
                        .chain(
                            state
                                .deferred
                                .iter()
                                .filter(|launch| launch.command.spec.recover)
                                .map(|launch| launch.entry.clone()),
                        )
                        .collect();
                    if let Err(e) = journal.write(entries) {
                        record_error(&mut state, e);
                    }
                }
                state.dirty = false;
            }
            if phase == SessionPhase::Closing
                && state.children.is_empty()
                && state.deferred.is_empty()
            {
                state.phase = SessionPhase::Closed;
                // Explicit handles may outlive the owner; release the cross-process journal lock.
                state.journal = None;
            }
            (
                state.phase == SessionPhase::Closed,
                state.children.is_empty()
                    && (state.deferred.is_empty()
                        || state.x11_startup_pending
                        || phase != SessionPhase::Ready),
            )
        };
        // Wakers may reenter session APIs. Never invoke them while holding the session mutex.
        for (completion, output) in completed {
            completion.finish(output);
        }
        if closed {
            break;
        }
        if idle {
            thread::park();
        } else {
            thread::park_timeout(Duration::from_millis(50));
        }
    }
}

fn record_error(state: &mut State, error: Error) {
    if state.errors.len() == 64 {
        state.errors.remove(0);
    }
    state.errors.push(error);
}

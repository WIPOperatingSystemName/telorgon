use super::{Error, Result, SessionHandle};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// Captured output is bounded per stream; excess bytes are drained and reported as truncated.
pub const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RestartPolicy {
    #[default]
    Never,
    /// Retry unsuccessful exits at most three times, with 1/2/4 second backoff. Successful exits
    /// are never restarted. This is opt-in for commands and the default for applications.
    OnFailure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stream {
    #[default]
    Inherit,
    Null,
    /// Capture stdout/stderr for ManagedChild::output. Not supported for stdin.
    Capture,
}

#[derive(Clone, Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct LaunchSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub directory: Option<PathBuf>,
    pub restart: RestartPolicy,
    pub recover: bool,
    pub application: Option<String>,
}

/// Inert builder. Executable and arguments are passed literally, without a shell.
#[derive(Clone)]
pub struct Command {
    pub(crate) spec: LaunchSpec,
    pub(crate) env: BTreeMap<OsString, Option<OsString>>,
    pub(crate) stdin: Stream,
    pub(crate) stdout: Stream,
    pub(crate) stderr: Stream,
    pub(crate) limit: usize,
    pub(crate) session: Option<SessionHandle>,
    pub(crate) activation_token: Option<String>,
    pub(crate) input: Option<Vec<u8>>,
}

impl Command {
    pub(crate) fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            spec: LaunchSpec {
                program: program.as_ref().into(),
                args: Vec::new(),
                directory: None,
                restart: RestartPolicy::Never,
                recover: false,
                application: None,
            },
            env: BTreeMap::new(),
            stdin: Stream::Inherit,
            stdout: Stream::Inherit,
            stderr: Stream::Inherit,
            limit: DEFAULT_OUTPUT_LIMIT,
            session: None,
            activation_token: None,
            input: None,
        }
    }
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.spec.args.push(arg.as_ref().into());
        self
    }
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.spec
            .args
            .extend(args.into_iter().map(|arg| arg.as_ref().into()));
        self
    }
    pub fn current_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.spec.directory = Some(path.as_ref().into());
        self
    }
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .insert(key.as_ref().into(), Some(value.as_ref().into()));
        self
    }
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.env.insert(key.as_ref().into(), None);
        self
    }
    pub fn stdin(mut self, mode: Stream) -> Self {
        self.stdin = mode;
        self
    }
    /// Feed bounded input on the supervisor thread, closing stdin after the last byte. Input is
    /// never persisted in the recovery journal. Limited to 8 MiB per launch.
    pub fn input(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.input = Some(bytes.into());
        self
    }
    pub fn stdout(mut self, mode: Stream) -> Self {
        self.stdout = mode;
        self
    }
    pub fn stderr(mut self, mode: Stream) -> Self {
        self.stderr = mode;
        self
    }
    pub fn output_limit(mut self, bytes: usize) -> Self {
        self.limit = bytes;
        self
    }
    pub fn restart(mut self, policy: RestartPolicy) -> Self {
        self.spec.restart = policy;
        self
    }
    /// Allow the launch to be offered after a host crash. Arguments may contain sensitive paths;
    /// environment overrides are not persisted and cannot be combined with recovery.
    pub fn recover(mut self, enabled: bool) -> Self {
        self.spec.recover = enabled;
        self
    }

    pub fn spawn(self) -> Result<ManagedChild> {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => super::current()?,
        };
        session.spawn(self)
    }

    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn()?.status().await
    }

    pub async fn output(mut self) -> Result<ProcessOutput> {
        self.stdin = Stream::Null;
        self.stdout = Stream::Capture;
        self.stderr = Stream::Capture;
        self.spawn()?.output().await
    }
}

pub(crate) struct Completion {
    pub pid: AtomicU32,
    state: Mutex<CompletionState>,
}

impl Completion {
    pub fn new(pid: u32) -> Self {
        Self {
            pid: AtomicU32::new(pid),
            state: Mutex::new(CompletionState {
                result: None,
                waiters: BTreeMap::new(),
                next_waiter: 0,
            }),
        }
    }
    pub fn finish(&self, result: Result<ProcessOutput>) {
        let waiters = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.result = Some(result.map(Arc::new));
            std::mem::take(&mut state.waiters)
        };
        for (_, waiter) in waiters {
            waiter.wake();
        }
    }
}

/// A session-supervised direct child. Its PID may change when OnFailure restarts it.
pub struct ManagedChild {
    pub(crate) completion: Arc<Completion>,
}

impl ManagedChild {
    /// Returns zero while startup readiness defers process creation. Never use
    /// zero as an OS process identifier; wait for a nonzero ID or completion.
    pub fn id(&self) -> u32 {
        self.completion.pid.load(Ordering::Acquire)
    }

    pub fn try_status(&self) -> Result<Option<ExitStatus>> {
        let state = self
            .completion
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match &state.result {
            Some(Ok(output)) => Ok(Some(output.status)),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(None),
        }
    }

    pub async fn status(&self) -> Result<ExitStatus> {
        Ok(self.wait().await?.status)
    }
    pub async fn output(&self) -> Result<ProcessOutput> {
        Ok((*self.wait().await?).clone())
    }

    fn wait(&self) -> ProcessWait {
        ProcessWait {
            completion: self.completion.clone(),
            registration: None,
        }
    }
}

struct CompletionState {
    result: Option<Result<Arc<ProcessOutput>>>,
    waiters: BTreeMap<u64, Waker>,
    next_waiter: u64,
}

// Registration belongs to the future, not the task's waker. Cancelling a wait releases its slot;
// moving the same future to another executor updates that slot without leaking old wakers.
struct ProcessWait {
    completion: Arc<Completion>,
    registration: Option<u64>,
}

impl Future for ProcessWait {
    type Output = Result<Arc<ProcessOutput>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut state = this
            .completion
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(result) = &state.result {
            return Poll::Ready(result.clone());
        }
        let id = match this.registration {
            Some(id) => id,
            None => {
                if state.waiters.len() >= 64 {
                    return Poll::Ready(Err(Error::Invalid(
                        "too many concurrent child waiters".into(),
                    )));
                }
                let id = state.next_waiter;
                let Some(next) = id.checked_add(1) else {
                    return Poll::Ready(Err(Error::Invalid("child waiter IDs exhausted".into())));
                };
                state.next_waiter = next;
                this.registration = Some(id);
                id
            }
        };
        state.waiters.insert(id, cx.waker().clone());
        Poll::Pending
    }
}
impl Drop for ProcessWait {
    fn drop(&mut self) {
        if let Some(id) = self.registration {
            self.completion
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .waiters
                .remove(&id);
        }
    }
}

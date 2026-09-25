use super::{registry::Core, *};
use std::{
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Autosave {
    path: PathBuf,
    debounce: Duration,
    max_delay: Duration,
}
impl Autosave {
    pub fn to(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            debounce: Duration::from_millis(500),
            max_delay: Duration::from_secs(5),
        }
    }
    pub fn debounce(mut self, delay: Duration) -> Self {
        self.debounce = delay;
        self
    }
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }
}

#[derive(Clone, Debug)]
pub struct SaveStatus {
    pub revision: u64,
    pub saved_revision: u64,
    pub dirty: bool,
    pub saving: bool,
    pub last_error: Option<String>,
}

pub(crate) struct Worker {
    config: Autosave,
    handle: JoinHandle<()>,
}

fn run(core: Arc<Core>, config: Autosave) {
    let mut retry_at = None;
    loop {
        let mut state = match core.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        loop {
            if state.stop {
                return;
            }
            if !state.dirty() {
                retry_at = None;
                state = match core.wake.wait(state) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                continue;
            }
            let first = state.first_change.unwrap_or_else(Instant::now);
            let last = state.last_change.unwrap_or(first);
            let mut deadline = (first + config.max_delay).min(last + config.debounce);
            if let Some(retry) = retry_at {
                deadline = deadline.max(retry);
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            state = match core.wake.wait_timeout(state, deadline - now) {
                Ok((s, _)) => s,
                Err(_) => return,
            };
        }
        drop(state);
        if core.save_file_inner(&config.path, true).is_err() {
            // A persistent I/O failure must not busy-loop or clear the dirty revision.
            retry_at = Some(Instant::now() + config.debounce.max(Duration::from_secs(1)));
        } else {
            retry_at = None;
        }
    }
}

impl Registry {
    pub fn save_status(&self) -> DataResult<SaveStatus> {
        let state = self.core.lock()?;
        Ok(SaveStatus {
            revision: state.revision,
            saved_revision: state.saved_revision,
            dirty: state.dirty(),
            saving: state.saving,
            last_error: state.last_error.clone(),
        })
    }

    pub fn enable_autosave(&self, mut config: Autosave) -> DataResult<()> {
        let day = Duration::from_secs(86400);
        if config.debounce.is_zero()
            || config.max_delay.is_zero()
            || config.debounce > day
            || config.max_delay > day
        {
            return Err(DataError::InvalidDelay);
        }
        let mut worker = self.worker.lock().map_err(|_| DataError::Poisoned)?;
        if worker.is_some() {
            return Err(DataError::AutosaveEnabled);
        }
        if !config.path.is_absolute() {
            config.path = std::env::current_dir()
                .map_err(|source| DataError::Io {
                    path: config.path.clone(),
                    source,
                })?
                .join(&config.path);
        }
        {
            let mut state = self.core.lock()?;
            state.stop = false;
            state.autosave_path = Some(config.path.clone());
        }
        let core = self.core.clone();
        let worker_config = config.clone();
        let handle = match thread::Builder::new()
            .name("telorgon-settings".into())
            .spawn(move || run(core, worker_config))
        {
            Ok(handle) => handle,
            Err(source) => {
                self.core.lock()?.autosave_path = None;
                return Err(DataError::Io {
                    path: config.path.clone(),
                    source,
                });
            }
        };
        *worker = Some(Worker { config, handle });
        Ok(())
    }

    /// Save to the autosave destination now, waiting for any older write to finish.
    pub fn flush(&self) -> DataResult<()> {
        let worker = self.worker.lock().map_err(|_| DataError::Poisoned)?;
        let config = &worker.as_ref().ok_or(DataError::AutosaveDisabled)?.config;
        self.core.save_file(&config.path)
    }

    /// Stop autosave without flushing. Use shutdown to flush before stopping.
    pub fn disable_autosave(&self) -> DataResult<()> {
        self.stop_worker(false)
    }

    /// Quiesce producers first. Stops the worker and saves one final snapshot; errors are returned.
    pub fn shutdown(&self) -> DataResult<()> {
        self.stop_worker(true)
    }

    fn stop_worker(&self, flush: bool) -> DataResult<()> {
        let mut slot = self.worker.lock().map_err(|_| DataError::Poisoned)?;
        let Some(worker) = slot.take() else {
            return Ok(());
        };
        self.core.lock()?.stop = true;
        self.core.wake.notify_all();
        worker
            .handle
            .join()
            .map_err(|_| DataError::WorkerPanicked)?;
        self.core.lock()?.autosave_path = None;
        if flush {
            self.core.save_file(&worker.config.path)?;
        }
        Ok(())
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        let _ = self.stop_worker(false);
    }
}

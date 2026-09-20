use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Wake, Waker};
use std::thread::{self, JoinHandle, ThreadId};

use crate::runtime::{
    Component, ComponentRuntimeDriver, LocalTask, RuntimeError, RuntimeResult, SendTask,
    TaskCancellation, TaskHost, ViewRuntime,
};

#[derive(Clone)]
struct HostWake {
    state: Arc<HostWakeState>,
}

struct HostWakeState {
    pending: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl HostWake {
    fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(HostWakeState {
                pending: AtomicBool::new(false),
                notify: Box::new(notify),
            }),
        }
    }

    fn signal(&self) {
        if !self.state.pending.swap(true, Ordering::AcqRel) {
            (self.state.notify)();
        }
    }

    fn begin_turn(&self) {
        self.state.pending.store(false, Ordering::Release);
    }
}

struct ExecutorCounters {
    local_spawned: AtomicUsize,
    local_polled: AtomicUsize,
    local_completed: AtomicUsize,
    local_cancelled: AtomicUsize,
    worker_spawned: AtomicUsize,
    worker_polled: AtomicUsize,
    worker_completed: AtomicUsize,
    worker_cancelled: AtomicUsize,
    stale_wakes: AtomicUsize,
}

impl ExecutorCounters {
    fn snapshot(&self) -> ManagedTaskDiagnostics {
        ManagedTaskDiagnostics {
            local_spawned: self.local_spawned.load(Ordering::Relaxed),
            local_polled: self.local_polled.load(Ordering::Relaxed),
            local_completed: self.local_completed.load(Ordering::Relaxed),
            local_cancelled: self.local_cancelled.load(Ordering::Relaxed),
            worker_spawned: self.worker_spawned.load(Ordering::Relaxed),
            worker_polled: self.worker_polled.load(Ordering::Relaxed),
            worker_completed: self.worker_completed.load(Ordering::Relaxed),
            worker_cancelled: self.worker_cancelled.load(Ordering::Relaxed),
            stale_wakes: self.stale_wakes.load(Ordering::Relaxed),
        }
    }
}

/// Cumulative evidence from one explicitly constructed managed task executor.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagedTaskDiagnostics {
    pub local_spawned: usize,
    pub local_polled: usize,
    pub local_completed: usize,
    pub local_cancelled: usize,
    pub worker_spawned: usize,
    pub worker_polled: usize,
    pub worker_completed: usize,
    pub worker_cancelled: usize,
    pub stale_wakes: usize,
}

/// Declared execution/threading behavior of the managed task adapter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ManagedTaskCapabilities {
    pub local_futures: bool,
    pub send_futures: bool,
    pub worker_threads: usize,
    pub detached_threads: bool,
}

impl Default for ManagedTaskCapabilities {
    fn default() -> Self {
        Self {
            local_futures: true,
            send_futures: true,
            worker_threads: 1,
            detached_threads: false,
        }
    }
}

/// Work consumed by one bounded owner-thread executor turn.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagedTaskPoll {
    pub polled: usize,
    pub completed: usize,
    pub cancelled: usize,
    pub stale: usize,
    pub remaining_ready: usize,
}

struct LocalReady {
    queue: Mutex<VecDeque<u64>>,
    wake: HostWake,
}

impl LocalReady {
    fn schedule(&self, id: u64) {
        lock(&self.queue).push_back(id);
        self.wake.signal();
    }
}

struct LocalControl {
    id: u64,
    active: AtomicBool,
    cancelled: AtomicBool,
    queued: AtomicBool,
    ready: Arc<LocalReady>,
}

impl LocalControl {
    fn schedule(&self) {
        if self.active.load(Ordering::Acquire) && !self.queued.swap(true, Ordering::AcqRel) {
            self.ready.schedule(self.id);
        }
    }

    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.schedule();
        }
    }
}

impl Wake for LocalControl {
    fn wake(self: Arc<Self>) {
        self.schedule();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.schedule();
    }
}

struct LocalCancellation(Arc<LocalControl>);

impl TaskCancellation for LocalCancellation {
    fn cancel(&mut self) {
        self.0.cancel();
    }
}

struct LocalSpawn {
    id: u64,
    task: LocalTask,
    control: Arc<LocalControl>,
}

struct LocalEntry {
    task: LocalTask,
    control: Arc<LocalControl>,
}

struct LocalShared {
    open: Arc<AtomicBool>,
    next_id: Cell<u64>,
    pending: RefCell<VecDeque<LocalSpawn>>,
    ready: Arc<LocalReady>,
    counters: Arc<ExecutorCounters>,
}

struct WorkerControl {
    id: u64,
    active: AtomicBool,
    cancelled: AtomicBool,
    queued: AtomicBool,
    sender: Sender<WorkerMessage>,
}

impl WorkerControl {
    fn schedule(&self) {
        if self.active.load(Ordering::Acquire)
            && !self.queued.swap(true, Ordering::AcqRel)
            && self.sender.send(WorkerMessage::Ready(self.id)).is_err()
        {
            self.active.store(false, Ordering::Release);
        }
    }

    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.schedule();
        }
    }
}

impl Wake for WorkerControl {
    fn wake(self: Arc<Self>) {
        self.schedule();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.schedule();
    }
}

struct WorkerCancellation(Arc<WorkerControl>);

impl TaskCancellation for WorkerCancellation {
    fn cancel(&mut self) {
        self.0.cancel();
    }
}

enum WorkerMessage {
    Spawn {
        id: u64,
        task: SendTask,
        control: Arc<WorkerControl>,
    },
    Ready(u64),
    Shutdown,
}

struct WorkerEntry {
    task: SendTask,
    control: Arc<WorkerControl>,
}

struct WorkerShared {
    open: Arc<AtomicBool>,
    next_id: AtomicU64,
    sender: Sender<WorkerMessage>,
    counters: Arc<ExecutorCounters>,
}

/// Runtime task-host capability backed by one owner-thread local queue and one declared worker.
///
/// Obtain this value from [`ManagedTaskExecutor::new`]. It is deliberately not a global executor;
/// dropping or shutting down the paired executor closes this capability.
pub struct ManagedTaskHost {
    local: Rc<LocalShared>,
    worker: Arc<WorkerShared>,
}

impl TaskHost for ManagedTaskHost {
    fn supports_local(&self) -> bool {
        self.local.open.load(Ordering::Acquire)
    }

    fn supports_send(&self) -> bool {
        self.worker.open.load(Ordering::Acquire)
    }

    fn spawn_local(&mut self, task: LocalTask) -> RuntimeResult<Box<dyn TaskCancellation>> {
        if !self.supports_local() {
            return Err(RuntimeError::new("managed local task executor is closed"));
        }
        let id = self.local.next_id.get().max(1);
        self.local.next_id.set(id.wrapping_add(1).max(1));
        let control = Arc::new(LocalControl {
            id,
            active: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            queued: AtomicBool::new(false),
            ready: self.local.ready.clone(),
        });
        self.local.pending.borrow_mut().push_back(LocalSpawn {
            id,
            task,
            control: control.clone(),
        });
        self.local
            .counters
            .local_spawned
            .fetch_add(1, Ordering::Relaxed);
        control.schedule();
        Ok(Box::new(LocalCancellation(control)))
    }

    fn spawn_send(&mut self, task: SendTask) -> RuntimeResult<Box<dyn TaskCancellation>> {
        if !self.supports_send() {
            return Err(RuntimeError::new("managed worker task executor is closed"));
        }
        let id = self.worker.next_id.fetch_add(1, Ordering::Relaxed).max(1);
        let control = Arc::new(WorkerControl {
            id,
            active: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            queued: AtomicBool::new(false),
            sender: self.worker.sender.clone(),
        });
        self.worker
            .sender
            .send(WorkerMessage::Spawn {
                id,
                task,
                control: control.clone(),
            })
            .map_err(|_| RuntimeError::new("managed worker task executor stopped"))?;
        self.worker
            .counters
            .worker_spawned
            .fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(WorkerCancellation(control)))
    }
}

/// Explicit managed executor driver paired with [`ManagedTaskHost`].
///
/// Construction starts one named worker for `Send` futures. Local futures remain retained and
/// polled only by [`poll_local`](Self::poll_local) on the constructing owner thread. Shutdown is
/// synchronous and joins the worker, so this type never leaves a detached background thread.
pub struct ManagedTaskExecutor {
    owner: ThreadId,
    local: Rc<LocalShared>,
    local_tasks: HashMap<u64, LocalEntry>,
    worker: Arc<WorkerShared>,
    worker_thread: Option<JoinHandle<()>>,
}

impl ManagedTaskExecutor {
    /// Creates an executor/host pair and a single explicitly owned worker thread.
    ///
    /// `wake` must enqueue a host turn. A Winit assembly supplies an `EventLoopProxy` callback;
    /// deterministic or embedded hosts may use their own completion queue.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> RuntimeResult<(Self, ManagedTaskHost)> {
        let local_open = Arc::new(AtomicBool::new(true));
        let worker_open = Arc::new(AtomicBool::new(true));
        let counters = Arc::new(ExecutorCounters {
            local_spawned: AtomicUsize::new(0),
            local_polled: AtomicUsize::new(0),
            local_completed: AtomicUsize::new(0),
            local_cancelled: AtomicUsize::new(0),
            worker_spawned: AtomicUsize::new(0),
            worker_polled: AtomicUsize::new(0),
            worker_completed: AtomicUsize::new(0),
            worker_cancelled: AtomicUsize::new(0),
            stale_wakes: AtomicUsize::new(0),
        });
        let local = Rc::new(LocalShared {
            open: local_open,
            next_id: Cell::new(1),
            pending: RefCell::new(VecDeque::new()),
            ready: Arc::new(LocalReady {
                queue: Mutex::new(VecDeque::new()),
                wake: HostWake::new(wake),
            }),
            counters: counters.clone(),
        });
        let (worker_sender, worker_receiver) = mpsc::channel();
        let worker = Arc::new(WorkerShared {
            open: worker_open,
            next_id: AtomicU64::new(1),
            sender: worker_sender,
            counters,
        });
        let worker_state = worker.clone();
        let worker_thread = thread::Builder::new()
            .name("telorgon-managed-task-worker".to_owned())
            .spawn(move || worker_main(worker_receiver, &worker_state))
            .map_err(|error| {
                RuntimeError::new(format!("managed task worker could not start: {error}"))
            })?;
        let host = ManagedTaskHost {
            local: local.clone(),
            worker: worker.clone(),
        };
        Ok((
            Self {
                owner: thread::current().id(),
                local,
                local_tasks: HashMap::new(),
                worker,
                worker_thread: Some(worker_thread),
            },
            host,
        ))
    }

    pub fn capabilities(&self) -> ManagedTaskCapabilities {
        ManagedTaskCapabilities::default()
    }

    /// Polls only ready local futures, up to `limit`, on the constructing owner thread.
    pub fn poll_local(&mut self, limit: usize) -> RuntimeResult<ManagedTaskPoll> {
        if limit == 0 {
            return Err(RuntimeError::new(
                "managed local task poll limit must be greater than zero",
            ));
        }
        if thread::current().id() != self.owner {
            return Err(RuntimeError::new(
                "managed local tasks must be polled on their owner thread",
            ));
        }
        self.local.ready.wake.begin_turn();
        self.ingest_local();
        let mut result = ManagedTaskPoll::default();
        while result.polled + result.cancelled + result.stale < limit {
            let Some(id) = lock(&self.local.ready.queue).pop_front() else {
                break;
            };
            let Some(entry) = self.local_tasks.get_mut(&id) else {
                result.stale += 1;
                self.local
                    .counters
                    .stale_wakes
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            };
            entry.control.queued.store(false, Ordering::Release);
            if entry.control.cancelled.load(Ordering::Acquire) {
                entry.control.active.store(false, Ordering::Release);
                self.local_tasks.remove(&id);
                result.cancelled += 1;
                self.local
                    .counters
                    .local_cancelled
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let waker = Waker::from(entry.control.clone());
            let mut context = Context::from_waker(&waker);
            result.polled += 1;
            self.local
                .counters
                .local_polled
                .fetch_add(1, Ordering::Relaxed);
            if entry.task.as_mut().poll(&mut context).is_ready() {
                entry.control.active.store(false, Ordering::Release);
                self.local_tasks.remove(&id);
                result.completed += 1;
                self.local
                    .counters
                    .local_completed
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        self.ingest_local();
        result.remaining_ready = lock(&self.local.ready.queue).len();
        if result.remaining_ready > 0 {
            self.local.ready.wake.signal();
        }
        Ok(result)
    }

    pub fn local_ready(&self) -> bool {
        !self.local.pending.borrow().is_empty() || !lock(&self.local.ready.queue).is_empty()
    }

    pub fn diagnostics(&self) -> ManagedTaskDiagnostics {
        self.local.counters.snapshot()
    }

    /// Closes both capabilities, drops local futures, and joins the worker.
    pub fn shutdown(&mut self) -> ManagedTaskDiagnostics {
        if self.local.open.swap(false, Ordering::AcqRel) {
            for (_, entry) in self.local_tasks.drain() {
                entry.control.cancelled.store(true, Ordering::Release);
                entry.control.active.store(false, Ordering::Release);
                self.local
                    .counters
                    .local_cancelled
                    .fetch_add(1, Ordering::Relaxed);
            }
            for spawn in self.local.pending.borrow_mut().drain(..) {
                spawn.control.cancelled.store(true, Ordering::Release);
                spawn.control.active.store(false, Ordering::Release);
                self.local
                    .counters
                    .local_cancelled
                    .fetch_add(1, Ordering::Relaxed);
            }
            lock(&self.local.ready.queue).clear();
        }
        if self.worker.open.swap(false, Ordering::AcqRel) {
            let _ = self.worker.sender.send(WorkerMessage::Shutdown);
        }
        if let Some(worker) = self.worker_thread.take() {
            let _ = worker.join();
        }
        self.diagnostics()
    }

    fn ingest_local(&mut self) {
        for spawn in self.local.pending.borrow_mut().drain(..) {
            self.local_tasks.insert(
                spawn.id,
                LocalEntry {
                    task: spawn.task,
                    control: spawn.control,
                },
            );
        }
    }
}

impl Drop for ManagedTaskExecutor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A managed component view that injects the default task host into the single runtime owner.
pub struct ManagedComponentRuntime<C: Component> {
    view: ViewRuntime<ComponentRuntimeDriver<C>>,
    tasks: ManagedTaskExecutor,
}

/// Work performed by one managed component task turn.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagedComponentTaskTurn {
    pub executor: ManagedTaskPoll,
    pub actions: usize,
}

impl<C: Component> ManagedComponentRuntime<C> {
    /// Creates a component runtime with working local and send task capabilities.
    pub fn new(component: C, wake: impl Fn() + Send + Sync + 'static) -> RuntimeResult<Self> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let (tasks, host) = ManagedTaskExecutor::new({
            let wake = wake.clone();
            move || wake()
        })?;
        let view =
            ViewRuntime::from_component_with_task_host_and_wake(component, host, move || wake())?;
        Ok(Self { view, tasks })
    }

    pub fn view(&self) -> &ViewRuntime<ComponentRuntimeDriver<C>> {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut ViewRuntime<ComponentRuntimeDriver<C>> {
        &mut self.view
    }

    /// Polls ready local futures and then drains one bounded runtime task-result turn.
    pub fn process_tasks(
        &mut self,
        local_poll_limit: usize,
    ) -> RuntimeResult<ManagedComponentTaskTurn> {
        let executor = self.tasks.poll_local(local_poll_limit)?;
        let actions = if self.view.task_results_ready() {
            self.view.process_component_task_results()?
        } else {
            0
        };
        Ok(ManagedComponentTaskTurn { executor, actions })
    }

    pub fn task_diagnostics(&self) -> ManagedTaskDiagnostics {
        self.tasks.diagnostics()
    }

    pub fn task_capabilities(&self) -> ManagedTaskCapabilities {
        self.tasks.capabilities()
    }

    /// Cancels runtime scopes before synchronously stopping the paired executor.
    pub fn shutdown_tasks(&mut self) -> ManagedTaskDiagnostics {
        self.view.shutdown_task_host();
        self.tasks.shutdown()
    }
}

impl<C: Component> Drop for ManagedComponentRuntime<C> {
    fn drop(&mut self) {
        self.shutdown_tasks();
    }
}

struct WorkerOpenGuard<'a>(&'a AtomicBool);

impl Drop for WorkerOpenGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn worker_main(receiver: mpsc::Receiver<WorkerMessage>, shared: &WorkerShared) {
    let _open = WorkerOpenGuard(&shared.open);
    let mut tasks = HashMap::<u64, WorkerEntry>::new();
    while let Ok(message) = receiver.recv() {
        match message {
            WorkerMessage::Spawn { id, task, control } => {
                control.queued.store(true, Ordering::Release);
                tasks.insert(id, WorkerEntry { task, control });
                poll_worker_task(&mut tasks, id, shared);
            }
            WorkerMessage::Ready(id) => poll_worker_task(&mut tasks, id, shared),
            WorkerMessage::Shutdown => {
                for (_, entry) in tasks.drain() {
                    entry.control.cancelled.store(true, Ordering::Release);
                    entry.control.active.store(false, Ordering::Release);
                    shared
                        .counters
                        .worker_cancelled
                        .fetch_add(1, Ordering::Relaxed);
                }
                break;
            }
        }
    }
}

fn poll_worker_task(tasks: &mut HashMap<u64, WorkerEntry>, id: u64, shared: &WorkerShared) {
    let Some(entry) = tasks.get_mut(&id) else {
        shared.counters.stale_wakes.fetch_add(1, Ordering::Relaxed);
        return;
    };
    entry.control.queued.store(false, Ordering::Release);
    if entry.control.cancelled.load(Ordering::Acquire) {
        entry.control.active.store(false, Ordering::Release);
        tasks.remove(&id);
        shared
            .counters
            .worker_cancelled
            .fetch_add(1, Ordering::Relaxed);
        return;
    }
    let waker = Waker::from(entry.control.clone());
    let mut context = Context::from_waker(&waker);
    shared
        .counters
        .worker_polled
        .fetch_add(1, Ordering::Relaxed);
    if entry.task.as_mut().poll(&mut context).is_ready() {
        entry.control.active.store(false, Ordering::Release);
        tasks.remove(&id);
        shared
            .counters
            .worker_completed
            .fetch_add(1, Ordering::Relaxed);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::mpsc::RecvTimeoutError;
    use std::task::Poll;
    use std::time::Duration;

    use crate::runtime::{CreateContext, State, Ui, UpdateContext};
    use crate::ui::{BoxStyle, LayoutStyle, UiRoot};

    struct ManagedComponent {
        observed: Rc<Cell<u32>>,
    }

    struct ManagedState(State<u32>);

    enum ManagedAction {
        StartLocal,
        StartSend,
        Complete(u32),
    }

    impl Component for ManagedComponent {
        type State = ManagedState;
        type Action = ManagedAction;

        fn create(&self, context: &mut CreateContext<'_>) -> Self::State {
            ManagedState(context.state(0))
        }

        fn mount(&self, _state: &Self::State, ui: &mut Ui<'_, '_, Self::Action>) -> UiRoot {
            ui.foundation()
                .root(BoxStyle::default(), LayoutStyle::default(), |_| {})
        }

        fn action(
            &self,
            state: &mut Self::State,
            action: Self::Action,
            context: &mut UpdateContext<'_, Self>,
        ) {
            match action {
                ManagedAction::StartLocal => {
                    context.spawn(async { ManagedAction::Complete(1) });
                }
                ManagedAction::StartSend => {
                    context.spawn_send(async { ManagedAction::Complete(2) });
                }
                ManagedAction::Complete(value) => {
                    context.set(state.0, value).unwrap();
                    self.observed.set(value);
                }
            }
        }
    }

    #[test]
    fn managed_component_runtime_polls_local_work_and_delivers_a_later_action() {
        let observed = Rc::new(Cell::new(0));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_count = wakes.clone();
        let mut runtime = ManagedComponentRuntime::new(
            ManagedComponent {
                observed: observed.clone(),
            },
            move || {
                wake_count.fetch_add(1, Ordering::Relaxed);
            },
        )
        .unwrap();
        assert_eq!(runtime.task_capabilities().worker_threads, 1);
        assert!(!runtime.task_capabilities().detached_threads);
        runtime
            .view_mut()
            .send_component_action(ManagedAction::StartLocal)
            .unwrap();
        assert_eq!(observed.get(), 0);
        let turn = runtime.process_tasks(1).unwrap();
        assert_eq!(turn.executor.polled, 1);
        assert_eq!(turn.executor.completed, 1);
        assert_eq!(turn.actions, 1);
        assert_eq!(observed.get(), 1);
        assert!(wakes.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn managed_worker_runs_send_work_and_shutdown_joins_it() {
        let observed = Rc::new(Cell::new(0));
        let (wake_sender, wake_receiver) = mpsc::channel();
        let mut runtime = ManagedComponentRuntime::new(
            ManagedComponent {
                observed: observed.clone(),
            },
            move || {
                let _ = wake_sender.send(());
            },
        )
        .unwrap();
        runtime
            .view_mut()
            .send_component_action(ManagedAction::StartSend)
            .unwrap();
        while !runtime.view().task_results_ready() {
            match wake_receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => panic!("managed worker did not wake the host"),
                Err(RecvTimeoutError::Disconnected) => panic!("managed wake channel closed"),
            }
        }
        let turn = runtime.process_tasks(1).unwrap();
        assert_eq!(turn.actions, 1);
        assert_eq!(observed.get(), 2);
        let diagnostics = runtime.shutdown_tasks();
        assert_eq!(diagnostics.worker_spawned, 1);
        assert_eq!(diagnostics.worker_completed, 1);
    }

    struct NeverReady {
        dropped: Arc<AtomicBool>,
    }

    impl Future for NeverReady {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for NeverReady {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    struct WorkerNeverReady {
        dropped: Option<mpsc::Sender<()>>,
    }

    impl Future for WorkerNeverReady {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for WorkerNeverReady {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    #[test]
    fn local_cancellation_is_budgeted_and_shutdown_closes_capabilities() {
        let (mut executor, mut host) = ManagedTaskExecutor::new(|| {}).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let mut cancellation = host
            .spawn_local(Box::pin(NeverReady {
                dropped: dropped.clone(),
            }))
            .unwrap();
        cancellation.cancel();
        let poll = executor.poll_local(1).unwrap();
        assert_eq!(poll.cancelled, 1);
        assert!(dropped.load(Ordering::Acquire));
        executor.shutdown();
        assert!(!host.supports_local());
        assert!(!host.supports_send());
        assert!(host.spawn_local(Box::pin(async {})).is_err());
    }

    #[test]
    fn worker_cancellation_drops_the_future_before_joined_shutdown() {
        let (mut executor, mut host) = ManagedTaskExecutor::new(|| {}).unwrap();
        let (dropped_sender, dropped_receiver) = mpsc::channel();
        let mut cancellation = host
            .spawn_send(Box::pin(WorkerNeverReady {
                dropped: Some(dropped_sender),
            }))
            .unwrap();
        cancellation.cancel();
        dropped_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker cancellation must drop its future");
        let diagnostics = executor.shutdown();
        assert_eq!(diagnostics.worker_cancelled, 1);
    }

    #[test]
    fn local_polling_obeys_the_owner_turn_budget_and_rewakes_for_leftovers() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_count = wakes.clone();
        let (mut executor, mut host) = ManagedTaskExecutor::new(move || {
            wake_count.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        for _ in 0..3 {
            host.spawn_local(Box::pin(async {})).unwrap();
        }
        let first = executor.poll_local(1).unwrap();
        assert_eq!(first.completed, 1);
        assert_eq!(first.remaining_ready, 2);
        let second = executor.poll_local(2).unwrap();
        assert_eq!(second.completed, 2);
        assert_eq!(second.remaining_ready, 0);
        assert!(wakes.load(Ordering::Relaxed) >= 2);
    }
}

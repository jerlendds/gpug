//! Bounded latest-request coordination for read-only interactive data queries.
//!
//! One thread services a fixed number of clients fairly. Pending requests coalesce
//! per client. This is deliberately unsuitable for graph mutations or durable saves,
//! which must not be silently superseded. Callbacks never run under the queue lock.
use std::fmt;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread::{self, JoinHandle, Thread};

/// A registered view. Generation prevents a disconnected/reused slot accepting
/// an old view's requests or results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientId {
    slot: usize,
    generation: u64,
}

/// Host-assigned data identity. Revisions are scoped to a dataset, not global.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WorkStamp {
    pub dataset: u64,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub client: ClientId,
    pub sequence: u64,
    pub stamp: WorkStamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    Busy,
    Closed,
    UnknownClient,
    ClientCapacity,
    SequenceExhausted,
    Poisoned,
    Panicked,
}
impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "query queue is briefly busy; retry without blocking",
            Self::Closed => "query worker is stopped",
            Self::UnknownClient => "unknown query client",
            Self::ClientCapacity => "query client capacity exhausted",
            Self::SequenceExhausted => "query sequence counter exhausted",
            Self::Poisoned => "query state lock is poisoned",
            Self::Panicked => "query handler panicked; worker stopped",
        })
    }
}
impl std::error::Error for WorkerError {}

/// Failed submission returns ownership of its input, including on Busy.
#[derive(Debug)]
pub struct SubmitError<I> {
    pub error: WorkerError,
    pub input: I,
}

#[derive(Debug)]
pub struct Completion<O> {
    pub ticket: Ticket,
    pub outcome: Result<O, WorkerError>,
}

/// Cooperative cancellation. Checked without locking; superseding a request,
/// cancelling/disconnecting its view, or shutting down makes it permanently true.
#[derive(Clone, Debug)]
pub struct Cancellation {
    current: Arc<AtomicU64>,
    sequence: u64,
    stopped: Arc<AtomicBool>,
}
impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
            || self.current.load(Ordering::Acquire) != self.sequence
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkerStats {
    pub submitted: u64,
    pub coalesced: u64,
    pub completed: u64,
    pub discarded: u64,
    pub pending: usize,
    pub ready: usize,
    pub in_flight: Option<Ticket>,
}
struct Request<I> {
    ticket: Ticket,
    input: I,
}
struct Client<I, O> {
    id: ClientId,
    current: Arc<AtomicU64>,
    latest: Option<Ticket>,
    pending: Option<Request<I>>,
    ready: Option<Completion<O>>,
}
struct State<I, O> {
    clients: Vec<Option<Client<I, O>>>,
    next_sequence: u64,
    cursor: usize,
    stats: WorkerStats,
}
impl<I, O> State<I, O> {
    fn sequence(&mut self) -> Result<u64, WorkerError> {
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(WorkerError::SequenceExhausted)?;
        Ok(self.next_sequence)
    }
    fn client(&mut self, id: ClientId) -> Result<&mut Client<I, O>, WorkerError> {
        self.clients
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|c| c.id == id)
            .ok_or(WorkerError::UnknownClient)
    }
}
struct Shared<I, O> {
    state: Mutex<State<I, O>>,
    stopped: Arc<AtomicBool>,
}
impl<I, O> Shared<I, O> {
    fn try_lock(&self) -> Result<MutexGuard<'_, State<I, O>>, WorkerError> {
        self.state.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => WorkerError::Busy,
            TryLockError::Poisoned(_) => WorkerError::Poisoned,
        })
    }
}

/// A single data worker with bounded per-view request/result slots.
///
/// UI-facing queue operations use try_lock and return Busy rather than waiting.
/// Payload destruction occurs outside locks but can still be costly: prefer Arc
/// inputs/results and retain ownership of large active snapshots in the host.
/// Bounds count objects, not bytes: one active request, one pending request per
/// client and one ready completion per client. The callback controls output size.
pub struct QueryWorker<I, O> {
    shared: Arc<Shared<I, O>>,
    thread: Thread,
    join: Option<JoinHandle<()>>,
}
impl<I, O> fmt::Debug for QueryWorker<I, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryWorker")
            .field("stopped", &self.is_stopped())
            .finish_non_exhaustive()
    }
}

impl<I: Send + 'static, O: Send + 'static> QueryWorker<I, O> {
    pub fn spawn(
        max_clients: usize,
        handler: impl FnMut(Ticket, I, &Cancellation) -> O + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with_notify(max_clients, handler, || {})
    }

    /// Notify runs on the worker thread after a current result is published, with
    /// no queue lock held. Use it to wake the UI through its supported dispatcher.
    /// A handler panic publishes an error if still current, then stops the worker.
    pub fn spawn_with_notify(
        max_clients: usize,
        handler: impl FnMut(Ticket, I, &Cancellation) -> O + Send + 'static,
        notify: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        if max_clients == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one client slot is required",
            ));
        }
        let mut clients = Vec::new();
        clients
            .try_reserve_exact(max_clients)
            .map_err(io::Error::other)?;
        clients.resize_with(max_clients, || None);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                clients,
                next_sequence: 0,
                cursor: 0,
                stats: WorkerStats::default(),
            }),
            stopped: Arc::new(AtomicBool::new(false)),
        });
        let worker = Arc::clone(&shared);
        let join = thread::Builder::new()
            .name("gpug-data-query".into())
            .spawn(move || run(worker, handler, notify))?;
        Ok(Self {
            shared,
            thread: join.thread().clone(),
            join: Some(join),
        })
    }
}

impl<I, O> QueryWorker<I, O> {
    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
    pub fn register(&self) -> Result<ClientId, WorkerError> {
        let mut state = self.shared.try_lock()?;
        if self.is_stopped() {
            return Err(WorkerError::Closed);
        }
        let slot = state
            .clients
            .iter()
            .position(Option::is_none)
            .ok_or(WorkerError::ClientCapacity)?;
        let id = ClientId {
            slot,
            generation: state.sequence()?,
        };
        state.clients[slot] = Some(Client {
            id,
            current: Arc::new(AtomicU64::new(0)),
            latest: None,
            pending: None,
            ready: None,
        });
        Ok(id)
    }

    pub fn submit(
        &self,
        id: ClientId,
        stamp: WorkStamp,
        input: I,
    ) -> Result<Ticket, SubmitError<I>> {
        let (ticket, old_request, old_result) = {
            let mut state = match self.shared.try_lock() {
                Ok(state) => state,
                Err(error) => return Err(SubmitError { error, input }),
            };
            if self.is_stopped() {
                return Err(SubmitError {
                    error: WorkerError::Closed,
                    input,
                });
            }
            if let Err(error) = state.client(id) {
                return Err(SubmitError { error, input });
            }
            let sequence = match state.sequence() {
                Ok(sequence) => sequence,
                Err(error) => return Err(SubmitError { error, input }),
            };
            let ticket = Ticket {
                client: id,
                sequence,
                stamp,
            };
            let client = state.client(id).expect("client validated under lock");
            client.current.store(sequence, Ordering::Release);
            client.latest = Some(ticket);
            let old_request = client.pending.replace(Request { ticket, input });
            let old_result = client.ready.take();
            state.stats.submitted = state.stats.submitted.saturating_add(1);
            if old_request.is_some() {
                state.stats.coalesced = state.stats.coalesced.saturating_add(1);
            }
            (ticket, old_request, old_result)
        };
        self.thread.unpark();
        drop((old_request, old_result));
        Ok(ticket)
    }

    /// Invalidate both active and pending work and remove any ready result.
    pub fn cancel(&self, id: ClientId) -> Result<(), WorkerError> {
        let old = {
            let mut state = self.shared.try_lock()?;
            let client = state.client(id)?;
            client.current.store(0, Ordering::Release);
            client.latest = None;
            (client.pending.take(), client.ready.take())
        };
        drop(old);
        Ok(())
    }
    pub fn unregister(&self, id: ClientId) -> Result<(), WorkerError> {
        let old = {
            let mut state = self.shared.try_lock()?;
            state.client(id)?.current.store(0, Ordering::Release);
            state.clients[id.slot].take()
        };
        drop(old);
        Ok(())
    }
    /// Take a completion that is current at the instant of polling. Compare its
    /// stamp against the host's current dataset before applying it to a view.
    pub fn try_take(&self, id: ClientId) -> Result<Option<Completion<O>>, WorkerError> {
        let mut state = self.shared.try_lock()?;
        let result = state.client(id)?.ready.take();
        let stopped = self.is_stopped();
        drop(state);
        if stopped
            && !matches!(
                &result,
                Some(Completion {
                    outcome: Err(WorkerError::Panicked),
                    ..
                })
            )
        {
            return Err(WorkerError::Closed);
        }
        Ok(result)
    }
    pub fn is_current(&self, ticket: Ticket) -> Result<bool, WorkerError> {
        Ok(!self.is_stopped()
            && self.shared.try_lock()?.client(ticket.client)?.latest == Some(ticket))
    }
    pub fn stats(&self) -> Result<WorkerStats, WorkerError> {
        let state = self.shared.try_lock()?;
        let mut stats = state.stats;
        for client in state.clients.iter().flatten() {
            stats.pending += usize::from(client.pending.is_some());
            stats.ready += usize::from(client.ready.is_some());
        }
        Ok(stats)
    }
    /// Signal shutdown without waiting. Non-cooperative callbacks can keep running
    /// until they return; successful results are not returned after shutdown.
    pub fn shutdown(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.thread.unpark();
    }
    /// Blocking join for application shutdown or tests, never the paint/input path.
    pub fn shutdown_and_join(mut self) -> thread::Result<()> {
        self.shutdown();
        self.join.take().expect("owned worker join handle").join()
    }
}
impl<I, O> Drop for QueryWorker<I, O> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ExitGuard(Arc<AtomicBool>);
impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn run<I: Send + 'static, O: Send + 'static>(
    shared: Arc<Shared<I, O>>,
    mut handler: impl FnMut(Ticket, I, &Cancellation) -> O,
    notify: impl Fn(),
) {
    let _exit = ExitGuard(Arc::clone(&shared.stopped));
    loop {
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            let capacity = state.clients.len();
            let mut job = None;
            for offset in 0..capacity {
                let slot = (state.cursor + offset) % capacity;
                if let Some(client) = state.clients[slot].as_mut() {
                    if let Some(request) = client.pending.take() {
                        let cancellation = Cancellation {
                            current: Arc::clone(&client.current),
                            sequence: request.ticket.sequence,
                            stopped: Arc::clone(&shared.stopped),
                        };
                        state.cursor = (slot + 1) % capacity;
                        state.stats.in_flight = Some(request.ticket);
                        job = Some((request, cancellation));
                        break;
                    }
                }
            }
            job
        };
        let Some((request, cancellation)) = job else {
            // unpark retains a token, so submissions/shutdown between the scan and
            // park cannot be lost. No periodic polling or condition-variable race.
            thread::park();
            continue;
        };
        let ticket = request.ticket;
        let output = if cancellation.is_cancelled() {
            None
        } else {
            Some(
                catch_unwind(AssertUnwindSafe(|| {
                    handler(ticket, request.input, &cancellation)
                }))
                .map_err(|_| WorkerError::Panicked),
            )
        };
        let panicked = matches!(output, Some(Err(WorkerError::Panicked)));
        let mut completion = output.map(|outcome| Completion { ticket, outcome });
        let publish = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            state.stats.in_flight = None;
            let current = !cancellation.is_cancelled()
                && state
                    .client(ticket.client)
                    .is_ok_and(|c| c.latest == Some(ticket));
            if current && completion.is_some() {
                state
                    .client(ticket.client)
                    .expect("checked current client")
                    .ready = completion.take();
                state.stats.completed = state.stats.completed.saturating_add(1);
                true
            } else {
                state.stats.discarded = state.stats.discarded.saturating_add(1);
                false
            }
        };
        drop(completion);
        if panicked {
            shared.stopped.store(true, Ordering::Release);
        }
        if publish {
            notify();
        }
        if panicked {
            return;
        }
    }
}

/// Immutable inputs for the built-in in-memory query worker. Assign a WorkStamp
/// corresponding to this snapshot when submitting; the worker does not invent a
/// global dataset/revision identity. Graph data is shared, never cloned per job.
#[derive(Debug)]
pub struct GraphQueryTask {
    pub graph: Arc<crate::GraphSnapshot>,
    pub query: Arc<crate::query::Query>,
    pub budget: crate::query::QueryBudget,
}

#[derive(Debug)]
pub struct OwnedQueryResult {
    pub nodes: Vec<crate::NodeId>,
    pub edges: Vec<crate::EdgeId>,
    pub depths: Vec<usize>,
    pub rows_examined: usize,
    pub adjacency_examined: usize,
    pub truncated: bool,
}

pub type GraphQueryWorker =
    QueryWorker<GraphQueryTask, Result<OwnedQueryResult, crate::GraphError>>;

/// Spawn a worker with reusable query/BFS scratch buffers and cooperative
/// cancellation. Only the bounded result IDs are copied out of the scratch space.
pub fn spawn_graph_query_worker(
    max_clients: usize,
    notify: impl Fn() + Send + 'static,
) -> io::Result<GraphQueryWorker> {
    let mut workspace = crate::query::QueryWorkspace::new();
    QueryWorker::spawn_with_notify(
        max_clients,
        move |_, task: GraphQueryTask, token| {
            let result =
                workspace.execute_cancellable(&task.graph, &task.query, task.budget, || {
                    token.is_cancelled()
                })?;
            Ok(OwnedQueryResult {
                nodes: result.nodes.to_vec(),
                edges: result.edges.to_vec(),
                depths: result.depths.to_vec(),
                rows_examined: result.rows_examined,
                adjacency_examined: result.adjacency_examined,
                truncated: result.truncated,
            })
        },
        notify,
    )
}

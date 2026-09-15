//! Transactional DuckDB persistence and analytical selection for data workers.
//!
//! One store owns one connection. Move it to a dedicated worker and keep all calls
//! outside GPUI render/input handlers. The `gpug_data` schema belongs to this
//! library; callers must not edit its tables through unrelated connections.
#![doc = include_str!("../docs/DUCKDB.md")]
use crate::query::{Query, QuerySource};
use crate::{Edge, EdgeId, GraphError, GraphSnapshot, Node, NodeId, WorldRect};
use duckdb::{params, params_from_iter, Connection, Transaction};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const VERSION: u32 = 1;
struct Database {
    connection: Mutex<Connection>,
    resources: StoreResources,
}
type SharedDatabase = Arc<Database>;
type DatabaseRegistry = Mutex<HashMap<PathBuf, Weak<Database>>>;
static DATABASES: OnceLock<DatabaseRegistry> = OnceLock::new();
const DDL: &str = "
CREATE SCHEMA IF NOT EXISTS gpug_data;
CREATE TABLE IF NOT EXISTS gpug_data.metadata (
    singleton BOOLEAN PRIMARY KEY CHECK (singleton),
    schema_version UINTEGER NOT NULL,
    revision UBIGINT NOT NULL
);
INSERT INTO gpug_data.metadata SELECT TRUE, 1, 0
WHERE NOT EXISTS (SELECT 1 FROM gpug_data.metadata);
CREATE TABLE IF NOT EXISTS gpug_data.nodes (
    ordinal UBIGINT NOT NULL UNIQUE, id UBIGINT PRIMARY KEY,
    x DOUBLE NOT NULL, y DOUBLE NOT NULL, width DOUBLE NOT NULL, height DOUBLE NOT NULL,
    label VARCHAR NOT NULL, renderer VARCHAR NOT NULL, hidden BOOLEAN NOT NULL
);
CREATE TABLE IF NOT EXISTS gpug_data.edges (
    ordinal UBIGINT NOT NULL UNIQUE, id UBIGINT PRIMARY KEY,
    source UBIGINT NOT NULL, target UBIGINT NOT NULL,
    source_handle VARCHAR, target_handle VARCHAR, weight DOUBLE NOT NULL
);";

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    RegistryPoisoned,
    InvalidResources,
    ResourceConflict {
        requested: StoreResources,
        active: StoreResources,
    },
    DuckDb(Box<duckdb::Error>),
    Graph(GraphError),
    SchemaVersion(u32),
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    RevisionExhausted,
    ReadLimit {
        table: &'static str,
        rows: u64,
        limit: usize,
    },
    UnsupportedQuery,
    ResultLimitTooLarge,
    Cancelled,
    Vortex(Box<crate::vortex_io::VortexGraphError>),
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "database path: {e}"),
            Self::RegistryPoisoned => write!(f, "database instance registry is poisoned"),
            Self::InvalidResources => write!(
                f,
                "resource limits require positive threads and memory bytes within i64 capacity"
            ),
            Self::ResourceConflict { requested, active } => write!(
                f,
                "database resource limits {active:?} differ from requested {requested:?}"
            ),
            Self::DuckDb(e) => write!(f, "duckdb: {e}"),
            Self::Graph(e) => write!(f, "invalid stored graph: {e}"),
            Self::SchemaVersion(v) => write!(f, "unsupported graph schema version {v}"),
            Self::RevisionConflict { expected, actual } => {
                write!(f, "expected graph revision {expected}, found {actual}")
            }
            Self::RevisionExhausted => write!(f, "graph revision counter exhausted"),
            Self::ReadLimit { table, rows, limit } => {
                write!(f, "{table} has {rows} rows, exceeding limit {limit}")
            }
            Self::UnsupportedQuery => write!(
                f,
                "neighborhood queries require a loaded graph and QueryWorkspace"
            ),
            Self::ResultLimitTooLarge => {
                write!(f, "result limit exceeds DuckDB signed LIMIT capacity")
            }
            Self::Cancelled => write!(f, "database query cancelled"),
            Self::Vortex(e) => write!(f, "vortex: {e}"),
        }
    }
}
impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::DuckDb(e) => Some(e.as_ref()),
            Self::Graph(e) => Some(e),
            Self::Vortex(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}
impl From<duckdb::Error> for StoreError {
    fn from(e: duckdb::Error) -> Self {
        Self::DuckDb(Box::new(e))
    }
}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<GraphError> for StoreError {
    fn from(e: GraphError) -> Self {
        Self::Graph(e)
    }
}
impl From<crate::vortex_io::VortexGraphError> for StoreError {
    fn from(e: crate::vortex_io::VortexGraphError) -> Self {
        Self::Vortex(Box::new(e))
    }
}

/// Whole-snapshot materialization limits, checked within the read transaction.
/// These bound rows, not string bytes or the cost of analytical SQL execution.
#[derive(Clone, Copy, Debug)]
pub struct StoreReadLimits {
    pub nodes: usize,
    pub edges: usize,
}
impl Default for StoreReadLimits {
    fn default() -> Self {
        Self {
            nodes: 1_000_000,
            edges: 10_000_000,
        }
    }
}

/// Limits shared by all worker connections to a live database instance.
/// The memory limit controls DuckDB's buffer manager, not total process memory;
/// Rust snapshots/results and some native allocations are outside it. Thread
/// limits control query parallelism, not CPU affinity or a wall-clock deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreResources {
    pub threads: u32,
    pub memory_limit_bytes: u64,
}
impl Default for StoreResources {
    fn default() -> Self {
        Self {
            threads: 2,
            memory_limit_bytes: 512 * 1024 * 1024,
        }
    }
}
impl StoreResources {
    fn validate(self) -> Result<(), StoreError> {
        if self.threads == 0
            || self.memory_limit_bytes == 0
            || self.memory_limit_bytes > i64::MAX as u64
        {
            return Err(StoreError::InvalidResources);
        }
        Ok(())
    }
    fn config(self) -> Result<duckdb::Config, StoreError> {
        self.validate()?;
        Ok(duckdb::Config::default()
            .threads(i64::from(self.threads))?
            .max_memory(&format!("{}B", self.memory_limit_bytes))?)
    }
}

#[derive(Debug)]
pub struct StoredGraph {
    pub revision: u64,
    pub graph: GraphSnapshot,
}

/// Exact matching IDs in saved input order, optionally truncated by the host cap.
#[derive(Debug)]
pub struct StoredSelection {
    pub revision: u64,
    pub nodes: Vec<NodeId>,
    pub edges: Vec<EdgeId>,
    pub truncated: bool,
}

/// Single-connection, synchronous data-worker backend. New stores contain an
/// empty graph at revision zero. Saves require the last observed revision.
pub struct DuckDbStore {
    connection: Connection,
    database: SharedDatabase,
}
impl fmt::Debug for DuckDbStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DuckDbStore").finish_non_exhaustive()
    }
}

impl DuckDbStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_resources(path, StoreResources::default())
    }
    /// Open with database-wide limits, before schema creation or any query.
    /// A live store for this canonical path must have identical limits; conflicting
    /// opens fail instead of changing another worker's resource policy.
    pub fn open_with_resources(
        path: impl AsRef<Path>,
        resources: StoreResources,
    ) -> Result<Self, StoreError> {
        resources.validate()?;
        let path = path.as_ref();
        if path == Path::new(":memory:") {
            return Self::in_memory_with_resources(resources);
        }
        let key = canonical_database_path(path)?;
        let mut databases = DATABASES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|_| StoreError::RegistryPoisoned)?;
        databases.retain(|_, database| database.strong_count() > 0);
        if let Some(database) = databases.get(&key).and_then(Weak::upgrade) {
            if database.resources != resources {
                return Err(StoreError::ResourceConflict {
                    requested: resources,
                    active: database.resources,
                });
            }
            return Self::connect(database);
        }
        let store = Self::initialize(
            Connection::open_with_flags(&key, resources.config()?)?,
            resources,
        )?;
        databases.insert(key, Arc::downgrade(&store.database));
        Ok(store)
    }
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::in_memory_with_resources(StoreResources::default())
    }
    pub fn in_memory_with_resources(resources: StoreResources) -> Result<Self, StoreError> {
        Self::initialize(
            Connection::open_in_memory_with_flags(resources.config()?)?,
            resources,
        )
    }
    /// Requested limits applied when this database instance was opened.
    pub fn resources(&self) -> StoreResources {
        self.database.resources
    }
    fn initialize(
        mut connection: Connection,
        resources: StoreResources,
    ) -> Result<Self, StoreError> {
        let tx = connection.transaction()?;
        tx.execute_batch(DDL)?;
        revision(&tx)?;
        tx.commit()?;
        Self::connect(Arc::new(Database {
            connection: Mutex::new(connection),
            resources,
        }))
    }
    fn connect(database: SharedDatabase) -> Result<Self, StoreError> {
        let connection = database
            .connection
            .lock()
            .map_err(|_| StoreError::RegistryPoisoned)?
            .try_clone()?;
        Ok(Self {
            connection,
            database,
        })
    }
    /// Create an independent worker connection to this same database instance,
    /// including an in-memory database. Transactions remain connection-local.
    pub fn try_clone(&self) -> Result<Self, StoreError> {
        Self::connect(Arc::clone(&self.database))
    }
    pub fn revision(&self) -> Result<u64, StoreError> {
        revision(&self.connection)
    }

    /// Replace both tables in one transaction using bulk appenders. A stale
    /// expected revision or any write error rolls the entire transaction back.
    /// A DuckDB concurrent-writer conflict may also be reported as a database error.
    pub fn save(
        &mut self,
        graph: &GraphSnapshot,
        expected_revision: u64,
    ) -> Result<u64, StoreError> {
        let tx = self.connection.transaction()?;
        let actual = revision(&tx)?;
        if actual != expected_revision {
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let next = actual.checked_add(1).ok_or(StoreError::RevisionExhausted)?;
        tx.execute_batch("DELETE FROM gpug_data.edges; DELETE FROM gpug_data.nodes;")?;
        {
            let mut nodes = tx.appender_to_db("nodes", "gpug_data")?;
            for (ordinal, node) in graph.nodes().iter().enumerate() {
                nodes.append_row(params![
                    ordinal as u64,
                    node.id.0,
                    node.bounds.x,
                    node.bounds.y,
                    node.bounds.width,
                    node.bounds.height,
                    node.label,
                    node.renderer,
                    node.hidden
                ])?;
            }
            // Do not rely on Drop to flush: its errors cannot be propagated.
            nodes.flush()?;
        }
        {
            let mut edges = tx.appender_to_db("edges", "gpug_data")?;
            for (ordinal, edge) in graph.edges().iter().enumerate() {
                edges.append_row(params![
                    ordinal as u64,
                    edge.id.0,
                    edge.source.0,
                    edge.target.0,
                    edge.source_handle,
                    edge.target_handle,
                    edge.weight
                ])?;
            }
            edges.flush()?;
        }
        tx.execute(
            "UPDATE gpug_data.metadata SET revision = ? WHERE singleton",
            params![next],
        )?;
        tx.commit()?;
        Ok(next)
    }

    /// Read metadata, counts and both relations from a single consistent database
    /// snapshot. Validate all graph invariants before returning any graph data.
    pub fn load(&mut self, limits: StoreReadLimits) -> Result<StoredGraph, StoreError> {
        let tx = self.connection.transaction()?;
        let revision = revision(&tx)?;
        let node_count = count(
            &tx,
            "SELECT count(*) FROM gpug_data.nodes",
            "nodes",
            limits.nodes,
        )?;
        let edge_count = count(
            &tx,
            "SELECT count(*) FROM gpug_data.edges",
            "edges",
            limits.edges,
        )?;
        let mut nodes = Vec::with_capacity(node_count);
        {
            let mut statement = tx.prepare("SELECT id, x, y, width, height, label, renderer, hidden FROM gpug_data.nodes ORDER BY ordinal")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                nodes.push(Node {
                    id: NodeId(row.get(0)?),
                    bounds: WorldRect {
                        x: row.get(1)?,
                        y: row.get(2)?,
                        width: row.get(3)?,
                        height: row.get(4)?,
                    },
                    label: row.get(5)?,
                    renderer: row.get(6)?,
                    hidden: row.get(7)?,
                });
            }
        }
        let mut edges = Vec::with_capacity(edge_count);
        {
            let mut statement = tx.prepare("SELECT id, source, target, source_handle, target_handle, weight FROM gpug_data.edges ORDER BY ordinal")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                edges.push(Edge {
                    id: EdgeId(row.get(0)?),
                    source: NodeId(row.get(1)?),
                    target: NodeId(row.get(2)?),
                    source_handle: row.get(3)?,
                    target_handle: row.get(4)?,
                    weight: row.get(5)?,
                });
            }
        }
        let graph = GraphSnapshot::new(nodes, edges)?;
        tx.commit()?;
        Ok(StoredGraph { revision, graph })
    }

    /// Execute node/edge queries directly in DuckDB with bound literal parameters.
    /// Includes endpoint attributes, degrees and CONTAINS. Neighborhood queries
    /// return UnsupportedQuery instead of implicitly loading the whole database.
    ///
    /// `max_results` bounds returned IDs, not database scan work or execution time.
    /// An extra matching row detects host truncation. Query LIMIT is honored.
    pub fn select(
        &mut self,
        query: &Query,
        max_results: usize,
    ) -> Result<StoredSelection, StoreError> {
        let (mut sql, mut values) = query.duckdb_select().ok_or(StoreError::UnsupportedQuery)?;
        let requested = query.limit().unwrap_or(usize::MAX);
        let limit = requested.min(max_results);
        let lookahead = requested > max_results;
        let fetch = limit
            .checked_add(usize::from(lookahead))
            .and_then(|v| i64::try_from(v).ok())
            .ok_or(StoreError::ResultLimitTooLarge)?;
        sql.push_str(" LIMIT ?");
        values.push(fetch.into());
        let tx = self.connection.transaction()?;
        let revision = revision(&tx)?;
        let mut result = StoredSelection {
            revision,
            nodes: Vec::new(),
            edges: Vec::new(),
            truncated: false,
        };
        {
            let mut statement = tx.prepare(&sql)?;
            let mut rows = statement.query(params_from_iter(values.iter()))?;
            while let Some(row) = rows.next()? {
                if result.nodes.len() + result.edges.len() == limit {
                    result.truncated = true;
                    break;
                }
                let id: u64 = row.get(0)?;
                if query.source() == QuerySource::Edges {
                    result.edges.push(EdgeId(id));
                } else {
                    result.nodes.push(NodeId(id));
                }
            }
        }
        tx.commit()?;
        Ok(result)
    }

    /// Select on a data worker with native interruption for superseded work.
    /// The predicate must be fast, thread-safe and remain true once cancelled;
    /// `|| token.is_cancelled()` accepts a QueryWorker cancellation token.
    /// A scoped watcher checks every 2 ms and interrupts this connection only.
    /// It is joined before return, preventing interrupts from leaking into the
    /// next operation. The interval is not a hard native-query latency bound.
    /// Cancelled calls publish no partial selection. Ordinary select has no watcher.
    pub fn select_cancellable(
        &mut self,
        query: &Query,
        max_results: usize,
        cancelled: impl Fn() -> bool + Sync,
    ) -> Result<StoredSelection, StoreError> {
        self.with_cancellation(cancelled, |store| store.select(query, max_results))
    }

    fn with_cancellation<T>(
        &mut self,
        cancelled: impl Fn() -> bool + Sync,
        operation: impl FnOnce(&mut Self) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::time::Duration;
        if cancelled() {
            return Err(StoreError::Cancelled);
        }
        let interrupt = self.connection.interrupt_handle();
        let observed = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let (done, receiver) = mpsc::channel::<()>();
            let predicate = &cancelled;
            let observed = &observed;
            scope.spawn(move || {
                while matches!(
                    receiver.recv_timeout(Duration::from_millis(2)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    if predicate() {
                        observed.store(true, Ordering::Release);
                        // Repeat until completion: cancellation may precede the
                        // native query starting, when a single interrupt is a no-op.
                        interrupt.interrupt();
                    }
                }
            });
            let result = operation(self);
            // Disconnect also happens during unwinding, so a panic cannot leave
            // a watcher running forever while thread::scope waits for it.
            drop(done);
            result
        });
        if observed.load(Ordering::Acquire) || cancelled() {
            Err(StoreError::Cancelled)
        } else {
            result
        }
    }

    /// Decode native Vortex files before opening the database write transaction.
    /// Invalid files cannot replace the previously committed graph.
    pub async fn import_vortex(
        &mut self,
        io: &crate::vortex_io::VortexGraphIo,
        files: crate::vortex_io::VortexGraphFiles,
        limits: crate::vortex_io::ReadLimits,
        expected_revision: u64,
    ) -> Result<u64, StoreError> {
        let graph = io.decode(files, limits).await?;
        self.save(&graph, expected_revision)
    }

    /// Export a consistent database snapshot as two native compressed Vortex files.
    /// The returned revision identifies the exported generation. Call on a worker:
    /// the snapshot load is synchronous even though compression/file encoding awaits.
    pub async fn export_vortex(
        &mut self,
        io: &crate::vortex_io::VortexGraphIo,
        limits: StoreReadLimits,
    ) -> Result<(u64, crate::vortex_io::VortexGraphFiles), StoreError> {
        let stored = self.load(limits)?;
        Ok((stored.revision, io.encode(&stored.graph).await?))
    }
}

fn canonical_database_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "database path has no file name",
                )
            })?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            Ok(parent.canonicalize()?.join(name))
        }
        Err(error) => Err(error),
    }
}

fn revision(connection: &Connection) -> Result<u64, StoreError> {
    let (version, revision): (u32, u64) = connection.query_row(
        "SELECT schema_version, revision FROM gpug_data.metadata WHERE singleton",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if version != VERSION {
        return Err(StoreError::SchemaVersion(version));
    }
    Ok(revision)
}
fn count(
    tx: &Transaction<'_>,
    sql: &str,
    table: &'static str,
    limit: usize,
) -> Result<usize, StoreError> {
    let rows: u64 = tx.query_row(sql, [], |row| row.get(0))?;
    if rows > limit as u64 {
        return Err(StoreError::ReadLimit { table, rows, limit });
    }
    usize::try_from(rows).map_err(|_| StoreError::ReadLimit { table, rows, limit })
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::Duration;

    #[test]
    fn native_cancel_is_scoped_and_connection_recovers() {
        let mut store = DuckDbStore::in_memory().unwrap();
        let mut other = store.try_clone().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = store.with_cancellation(
                || worker_cancel.load(Ordering::Acquire),
                |store| {
                    started_tx.send(()).unwrap();
                    let tx = store.connection.transaction()?;
                    let _: f64 = tx.query_row(
                        "SELECT sum(sin(i::DOUBLE)) FROM range(1000000000000) t(i)",
                        [],
                        |row| row.get(0),
                    )?;
                    tx.commit()?;
                    Ok(())
                },
            );
            finished_tx.send((store, result)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        cancel.store(true, Ordering::Release);
        let query = Query::parse("NODES").unwrap();
        // Interrupting one connection must not cancel another connection.
        assert!(other.select(&query, 10).unwrap().nodes.is_empty());
        let (mut store, result) = finished_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("native query must stop");
        worker.join().unwrap();
        assert!(matches!(result, Err(StoreError::Cancelled)));
        // The interrupted read transaction rolls back, and its watcher has
        // joined before a new transaction begins on this same connection.
        assert_eq!(store.revision().unwrap(), 0);
        assert!(store
            .select_cancellable(&query, 10, || false)
            .unwrap()
            .nodes
            .is_empty());
        let graph = GraphSnapshot::new(vec![Node::new(7, WorldRect::default())], vec![]).unwrap();
        assert_eq!(store.save(&graph, 0).unwrap(), 1);
        assert_eq!(store.select(&query, 10).unwrap().nodes, vec![NodeId(7)]);
    }

    #[test]
    fn precancelled_call_does_not_start_operation() {
        let mut store = DuckDbStore::in_memory().unwrap();
        let result: Result<(), StoreError> =
            store.with_cancellation(|| true, |_| panic!("must not run"));
        assert!(matches!(result, Err(StoreError::Cancelled)));
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn native_engine_observes_limits_on_each_connection() {
        let resources = StoreResources {
            threads: 1,
            memory_limit_bytes: 256 * 1024 * 1024,
        };
        let store = DuckDbStore::in_memory_with_resources(resources).unwrap();
        for connection in [&store.connection, &store.try_clone().unwrap().connection] {
            let (threads, memory): (i64, String) = connection
                .query_row(
                    "SELECT current_setting('threads'), current_setting('memory_limit')",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(threads, 1);
            assert_eq!(memory, "256.0 MiB");
        }
    }
}

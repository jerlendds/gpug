# gpug-data

Framework-neutral data and analytics crate for GPUG.
Vortex is the canonical in-memory backing model. DuckDB remains opt-in;
the core has no GPUI dependency and opens no database during graph queries.

Implemented:

- Immutable validated snapshots, stable u64 IDs, directed connections with handles.
- Atomic copy-on-write transactions, cascading deletion, induced subgraphs.
- Dense u32 endpoint columns and incoming/outgoing compressed sparse row indexes.
- Reusable, work-budgeted neighborhood BFS with hop distances and explicit truncation.
- Reusable minimum-hop paths with original edge IDs, budgets and cancellation.
- In/out degree and weakly connected components, including hidden nodes.
- Canonical native Vortex arrays, compressed files, and filtered/projected scans.
- Compiled graph queries with typed filters, endpoint joins, traversal, and budgets.
- Optional DuckDB transactional storage, revision checks, and parameterized queries.
- Bounded query workers with per-view coalescing, cancellation and stale-result rejection.
- GPUG-owned bridge preserving identities, renderer keys and measured geometry.
- Snapshot-bound spatial queries with conservative bounds and explicit work limits.
- Bounded node LOD plans with disjoint source membership and aggregate fallbacks.
- Exact directed aggregate connectivity, partition caching and source drill-down.
- Focused custom-node priority, including overlapping sources within hard budgets.
- Connection-local DuckDB cancellation for superseded worker selections.

```rust
use gpug_data::*;
let graph = GraphSnapshot::new(
    vec![Node::new(1, WorldRect::default()), Node::new(2, WorldRect::default())],
    vec![Edge::new(10, 1, 2)],
)?;
let mut search = SearchWorkspace::new();
let neighbors = search.neighborhood(
    &graph, NodeId(1), Direction::Outgoing, SearchBudget::default(),
)?;
assert_eq!(neighbors.nodes, &[NodeId(1), NodeId(2)]);
# Ok::<(), GraphError>(())
```

Snapshot construction, transactions, and whole-graph analytics belong on workers.
Share completed snapshots with `Arc`. Dense indices are valid only for their
snapshot; persistent references must use external IDs. Geometry uses absolute
world-space top-left bounds; a GPUG adapter must resolve parent/origin geometry.

## Shortest paths

`PathWorkspace::shortest_path` finds a minimum-hop path through the original
topology, including hidden nodes. Set `SearchBudget::max_hops` explicitly (its
default is one hop). `PathStatus::Found` supplies ordered node and edge IDs;
`NotFoundWithinHops` means no path within that hop limit; `Stopped` makes no
reachability claim. Cancelled or budget-stopped searches never return a partial
path. `Incoming` and `Both` can traverse an edge against its direction. Weights
do not affect this minimum-hop algorithm; weighted shortest paths are not yet
implemented. Run the search on a data worker and reuse its workspace.

## Query language

Use `query::Query::parse` and a reusable `query::QueryWorkspace` for node/edge
filters and neighborhood queries. See [QUERY_LANGUAGE.md](docs/QUERY_LANGUAGE.md) for
the grammar, endpoint attributes, NULL semantics, work budgets, and Vortex pushdown.

## Viewport queries

`spatial::SpatialIndex` indexes immutable node bounds and shares its source
snapshot. Reuse `SpatialWorkspace` to query viewports with explicit branch, node
and result budgets. An incomplete result must not be used as a complete culling
mask. See [SPATIAL.md](docs/SPATIAL.md) for semantics, limits and benchmarks.
`LodWorkspace` retains aggregates when detail cannot fit, preserving source
coverage. `ConnectionCache` retains exact directed group counts and edge membership
across repeated partitions. `prepare_focused` prioritizes one visible source while
preserving the remaining aggregate membership within hard budgets.

## GPUG views

GPUG depends on this crate and exposes it as `gpug::analysis`. Use
`gpug::gpug_bridge` and `cargo run -p gpug --example data_flow`.
See the [GPUG integration guide](docs/GPUG.md).
The `aggregate_graph` example combines worker planning, connection summaries,
paginated source inspection and custom flow cards; append `-- --overlap` to test
focused content with coincident nodes.
The bridge converts analytical query results into GPUG nodes/edges and exports
measured editor geometry. Host renderer registrations supply custom content;
near-maximum zoom relaxes its size cutoff while retaining the content budget.

## Interactive workers

`worker::spawn_graph_query_worker` shares immutable snapshots and compiled queries,
reuses scratch buffers, and cooperatively stops superseded work. Queue operations
return `Busy` instead of waiting on a lock. See [WORKERS.md](docs/WORKERS.md) for host
integration, request identities, bounds and shutdown semantics.

## Vortex IO

Use `gpug_data::vortex_io::VortexGraphIo` without a feature flag. Vortex 0.86.1
is required. The old `vortex` feature remains a no-op compatibility alias.
Snapshots own canonical Vortex tables; the legacy `nodes()`/`edges()` slice APIs
materialize and cache row adapters on first access. Export shares the canonical
arrays. Dense endpoints and CSR adjacency use native Vortex buffers.

`graph_arrays` exports native struct arrays with separate node and edge relations.
`encode` returns two standalone compressed `.vortex` files. `decode` validates
both schemas and row limits before materializing them, then checks graph invariants.
`read_paths` reopens a pair of immutable files. `scan_path` supports Vortex filter
and projection expressions and range reads, so a position-only query can omit
labels and renderer data. `scan` provides the same interface for in-memory bytes.

The v1 schemas retain u64 stable IDs, f64 absolute bounds, labels, renderer keys,
visibility, edge weights, and nullable handles. Absent handles and empty strings
remain distinct. Schema matching is strict, including field order and nullability.

These APIs materialize their output and belong on a dedicated data worker. The
default whole-graph read limits are 1M nodes/10M edges; callers can configure them.
Row limits do not bound string payload bytes. `scan`/`scan_path` are lower-level
analytical APIs without those whole-graph row limits. The two encoded files must
be published together as one immutable generation. DuckDB import/export preserves
generations through a transactional store.

## DuckDB

Enable `duckdb` for a matching system library or `duckdb-bundled` to build the
native engine from source. `duckdb_store::DuckDbStore` supports atomic saves,
validated loads, parameterized node/edge queries, revision checks and, with the
`vortex` feature, native Vortex import/export. See [DUCKDB.md](docs/DUCKDB.md) for usage,
connection ownership, limits, and verification commands.

## Validation

`cargo test -p gpug-data --offline`

`cargo test -p gpug-data --offline --features vortex`

`cargo test -p gpug -p gpug-data --offline --features duckdb`

`cargo clippy -p gpug-data --offline --all-targets -- -D warnings`

`cargo clippy -p gpug-data --offline --features vortex --all-targets -- -D warnings`

`cargo run -p gpug-data --offline --release --example neighborhood_benchmark`

The exhaustive BFS test compares against independent edge-list semantics across
all 512 directed three-node graphs, all seeds/directions, and hop limits 0–3.
The benchmark measures warm one-hop chain traversal against full edge scans.
It does not measure rendering, storage, dense/hub graphs, or end-to-end UI latency.

`../gpug/tests/persisted_view.rs` exercises native Vortex encoding, persistent DuckDB
import/reopening, SQL selection, stamped worker traversal and focused GPUG view
conversion together. It verifies source identities, custom renderer/label retention
and exact edge provenance. It constructs view data; desktop painting is verified
separately. [NAVIGATION.md](docs/NAVIGATION.md) records measured camera-preparation costs
and controlled cache/lookup comparisons through one million nodes.

See [IMPLEMENTATION.md](docs/IMPLEMENTATION.md) for implementation history and acceptance evidence.
See [ACCEPTANCE.md](docs/ACCEPTANCE.md) for the requirement-by-requirement delivery audit,
evidence boundaries. See [LIVE_REPLAY.md](docs/LIVE_REPLAY.md) for the million-node
live measurements and replay commands.
See [PROFILING.md](docs/PROFILING.md) for the VegaProf-inspired profiling plan and current measurement limits.

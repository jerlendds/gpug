# Vortex model, analysis and optional persistence

GPUG depends on `crates/data` and re-exports it as `gpug::analysis`. Applications
need only the `gpug` dependency. Vortex is required. DuckDB is disabled by default,
and constructing, editing, rendering or querying a graph never opens a database.

```toml
[dependencies]
gpug = { path = "path/to/crates/gpug" }
# Add features = ["duckdb"] for a matching system DuckDB library,
# or features = ["duckdb-bundled"] to compile the native engine.
```

## Model and rendering

`GraphSnapshot` owns immutable canonical Vortex node/edge tables, stable IDs, dense
endpoint buffers, and incoming/outgoing CSR adjacency. `vortex_tables()` exposes
shared arrays directly, including for encoding. Existing `nodes()` and `edges()`
slice APIs lazily materialize a cached row adapter; their first use can cost
O(V + E) and retain an additional row representation. Snapshot construction,
materialization and whole-graph analysis belong on data workers.

The mutable GPUG editor retains presentation specifications and measured runtime
state. Its numeric geometry, render topology and force-layout adjacency use native
Vortex buffers. Pointer updates and layout mutate these working buffers directly;
they do not rebuild immutable tables, compress data or perform IO per frame.
Snapshots are explicit captures of that working state. Camera changes do not
change analytical geometry. The renderer's revision and culling rules still apply.

Vortex backing does not by itself establish a rendering speedup: GPUG already used
dense arrays and CSR adjacency. See the backing benchmark for a controlled
comparison; no compressed scalar access is added to paint loops.

## Construct and query a one-off graph

```rust
use gpug::{analysis as data, EditorModel, GraphOwnership};
use data::query::{Query, QueryBudget, QueryWorkspace};

let source = data::GraphSnapshot::new(
    vec![data::Node::new(1, data::WorldRect::default()),
         data::Node::new(2, data::WorldRect::default())],
    vec![data::Edge::new(10, 1, 2)],
)?;
let mut query = QueryWorkspace::new();
let selected = query.execute(&source,
    &Query::parse("NEIGHBORS 1 OUTGOING HOPS 2")?, QueryBudget::default())?;
assert!(!selected.truncated);

let editor = EditorModel::from_snapshot(&source, GraphOwnership::Internal)?;
// In a GPUI App context:
// let graph = gpug::Graph::builder().snapshot(&source)?.build(cx)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Reuse `SearchWorkspace`, `PathWorkspace` and `QueryWorkspace`. Query results refer
to the source snapshot's IDs. Hidden nodes participate in analysis unless filtered.
An incomplete result must not be used as a complete culling mask. For live queries,
use `analysis::worker` and validate request tickets and source stamps before
publication; saves and mutations need an ordered queue, not a coalescing query queue.

## Capture edits and save explicitly

`Graph::capture_snapshot` and `EditorModel::capture_snapshot` copy analytical fields
and measured absolute geometry on their owner thread. Move the owned capture to a
worker and call `into_snapshot()` to validate rows and build Vortex tables/indexes.
Capture itself is O(V + E), not a bounded per-frame operation. `snapshot()` combines
both steps synchronously for small graphs and worker-owned editors.

With the `duckdb` feature, on a data worker:

```rust
use gpug::analysis::duckdb_store::{DuckDbStore, StoreReadLimits};
// `capture` was obtained from graph.capture_snapshot()? on the UI thread.
let snapshot = capture.into_snapshot()?;
let mut database = DuckDbStore::open("graph.duckdb")?;
let loaded = database.load(StoreReadLimits::default())?;
// Resolve any application-level changes against `loaded` before saving.
let revision = database.save(&snapshot, loaded.revision)?;
```

Retain the revision obtained when loading/saving a generation. A stale save fails
atomically; do not blindly load a newer revision just to overwrite concurrent edits.
One database connection belongs to one worker. Use the store's resource limits to
bound database thread and buffer usage. Saving is never automatic.

Load and convert on a worker with `GraphData::try_from(&loaded.graph)`, then install
with `Graph::set_data` on the UI thread. For small inputs, `Graph::set_snapshot`
combines conversion and installation. `EditorModel::replace_snapshot` is the
framework-neutral equivalent. Both validate before replacement and retain the
ownership mode. Installation still validates/rebuilds GPUG state on the UI thread.

Internal edits appear in subsequent captures after commit. External ownership
queues edit proposals; captures reflect the host-supplied committed state until
replacement arrives. Existing middleware and atomic node/edge changes remain the
mutation path. Replacing a snapshot is host publication, not a user-edit event.

## Preserved fields and migration

Stable u64 node/edge IDs, nullable handles, labels, renderer keys, visibility,
weights and measured world-space bounds survive the bridge. Geometry narrows
from f64 to f32 with overflow rejection. Parent-relative origins are resolved
to absolute top-left bounds; imported nodes are flat with zero origin.

Arbitrary metadata, hierarchy, selection, capabilities, callbacks, viewport and
custom UI state are not part of the analytical storage schema. Keep those in host
state. The bridge is not a complete editor-session format. Reserved metadata keys
are documented in `gpug::gpug_bridge`; malformed analytical weights are errors.

The old `gpug-data/gpug` feature and bridge moved to GPUG to remove the dependency
cycle. Import `gpug::gpug_bridge` and `gpug::analysis` instead. The `vortex` feature
is now a no-op compatibility alias in both crates; Vortex cannot be disabled.
`NodeColumns` numeric fields now use `analysis::column::Column<T>` rather than
`Vec<T>`. Indexing, iteration and mutable slice access remain available; use
`.to_vec()` when an owned Vec is needed.
Run the moved examples with `cargo run -p gpug --example data_flow` or
`cargo run -p gpug --example aggregate_graph`.

## Benchmark

See the [measured comparison](../benchmarks/backing-final/README.md): Vortex showed
a modest 100k-node camera benefit, with no consistent advantage in smaller graphs.

```sh
cargo build -p gpug --release --features benchmark-backends --example backing_benchmark
python3 tools/benchmark_backends.py --output benchmarks/backing-final --samples 60 --pairs 3
```

`benchmark-backends` adds an experimental Vec comparison selected once per process
by `GPUG_BENCH_BACKEND=vec`. Normal builds always use Vortex. Both benchmark modes
share algorithms, camera poses, workload, compiler settings and renderer. Vortex
remains a build dependency even when running the Vec baseline. `SOURCE` and
`INSTALL` report startup milliseconds separately; `RESULT` reports backend,
workload, nodes, sampled frames, CPU paint-completion FPS, and p50/p95/p99 frame
intervals in milliseconds. This cadence includes desktop scheduling and is not
a measurement of GPU presentation. Use alternating paired runs on the same window
and display, with no competing builds. Set `GPUG_BENCH_WORKLOAD=hub` for skewed
connectivity instead of local grid connections.

The harness alternates backends for 10k/30k-edge and 100k/300k-edge camera sweeps,
plus a 10k-node layout replay with one deterministic step per frame. Each process
runs in a separate systemd user scope with a 6 GiB memory limit and no swap, outside
the editor's scope. It checks matching viewport dimensions and final graph checksums.
Use `--case 100000`, `--case 10000`, or `--case layout` to select cases.

The visible CPU FPS meter counts painted frames independently of the layout step
counter, including when simulation is stopped. The native Blade upload allocator
is patched to grow capacities geometrically and reclaim completed undersized
buffers; see [patch provenance](../vendor/blade-util/UPSTREAM.md).

Connection handles and edge arrowheads follow the node content size threshold
(default 18 screen pixels high), independent of the content budget. Selected-node
connection handles follow the same policy. Full detail returns near the configured
maximum zoom. Active connection/reconnection feedback remains visible. Handles use
analytic rounded quads instead of triangle fans when visible.

## Validation for this integration

- 203 native library/integration tests passed with DuckDB and benchmark backends enabled.
- The FPS meter regression passed; it advances independently of layout ticks.
- 48 data tests passed in the explicit Vec baseline process.
- Default-feature GPUG all-target checking, formatting and whitespace checks passed.
- Clippy passed with existing `redundant_closure`, `items_after_test_module`,
  `needless_range_loop` and `manual_is_multiple_of` warnings explicitly allowed
  for the check. No source lint policy was changed.
- Browser validation is unverified: `python3 web/build.py --check --debug --offline`
  fails while loading the prepared GPUI dependency because Cargo cannot inherit
  its workspace edition. The native results above do not establish WASM support.

# Performance architecture

The default large-graph path uses 10,000 nodes, three local ring neighbors per
side, and independently sampled long-range shortcuts with probability
`0.00001`—approximately 30,500 edges total.

The seeded initial positions follow the ring with slight radial jitter, so the
network structure is legible before layout starts. Stopping layout automatically
fits the settled result to the current viewport.

- Graph positions and layout forces are contiguous arrays.
- Stable domain IDs are compiled once to dense topology indices.
- A Barnes–Hut quadtree approximates repulsion in O(n log n).
- CSR adjacency permits lock-free parallel attraction by node.
- ForceAtlas2 adaptive speed uses swinging and effective traction.
- One GPUI graph entity replaces one entity per node.
- Nodes and edges are emitted as batched paths with viewport culling.
- Interactive edge LOD bounds per-frame geometry; paused mode is full detail.

Run the repeatable review loop:

```bash
./scripts/review-optimize-loop.sh my-iteration
```

It checks formatting, compilation, tests, and Clippy; benchmarks the standard
workload; captures initial, interactive, and paused full-detail screenshots;
and generates normalized benchmark deltas and RMSE screenshot differences.
Benchmarks report the median of three paired samples by default; override with
`GPUG_BENCH_SAMPLES`.

See the repository-level `PERFORMANCE.md` for measured results and Gephi source
references.

## Pointer input and gesture capture

Graph picking starts on left mouse-down. Ordinary hover does not scan graph nodes,
handles or edges, publish graph events, or request a graph redraw. The canvas keeps
its grab cursor until a gesture is captured. Mouse release clears gesture feedback
without running another hover pick. GPUI still performs its normal element event
routing; this policy concerns graph geometry tests.

Canvas pan and node drag use the captured gesture instead of re-picking items.
Connection/reconnection drags query drop handles only while the button is held.
Node motion stores the latest world-space pointer position and applies one validated
movement batch before each paint. Release drains any pending movement before the
final `dragging=false` batch. Captured node IDs remain stable across host edits.
Middleware, external ownership and atomic model validation remain in the executor;
the optimization changes transient update frequency, not the mutation contract.

`GPUG_PROFILE=1` now includes pointer, mouse-down, handle, reconnection and drag-edit
timings. Pointer p50/p95/p99 reports also flush every 512 samples during a paint
stall, when the normal frame reporter cannot run.

See the [interaction measurements](../benchmarks/interaction-final/README.md) for
reproduction, limits, and remaining drag transaction cost.

### Live Vortex geometry reads

Node-body and edge-body picking read the live Vortex-backed numeric columns.
Handle and reconnection checks now do the same for measured position and size;
reconnection uses compiled dense endpoints when topology is current. Stale indices
fall back to stable-ID runtime lookups. Rich node capabilities and renderer behavior
remain in specifications, while immutable analytical snapshots are reserved for
explicit analysis/persistence work rather than rebuilding them during gestures.

The [paired geometry-access benchmark](../benchmarks/interaction-geometry/README.md)
measured about 54–56% lower handle/reconnection cost, and about 7.6% lower median
whole mouse-down latency. The optional `benchmark-interaction` feature retains
`GPUG_PICK_GEOMETRY=rows` for reproducing the object/hash lookup comparison.

## 50,000-node profiling

The [50k optimization report](../benchmarks/50k-final/README.md) separates animated
follow/interaction from actual pan/zoom and full-edge drawing (162,512 edges).
The 60 FPS objective is measured against a 16.7 ms frame interval; existing LOD
results must not be represented as drawing every edge.

Graph-owned snapshots now validate scalar transactions using the changed IDs;
structural edits and publicly mutable standalone EditorModel snapshots retain
full projected-graph validation. Selection-only batches preserve atomic events,
external overlays and custom geometry invalidation without rebuilding topology.
Selection publication uses revisions, so a persistent dirty overlay bit cannot
republish selection and rebuild appearance arrays on every drag update.

Edge appearance caches do not depend on zoom. Built-in node appearance caches use
the clamped pixel radius, while custom callbacks remain sensitive to exact zoom.
Fit/follow bounds use the synchronized measured Vortex columns.

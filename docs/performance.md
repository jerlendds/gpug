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

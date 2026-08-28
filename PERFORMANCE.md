# GPUG large-graph performance

The standard stress workload is now a deterministic small-world graph with
10,000 nodes. Each node is connected to its three nearest ring neighbors in
each direction, and every other node pair has a `0.00001` chance of receiving
a long-range shortcut. Seed 42 produces 30,507 unique edges: 30,000 local ring
edges plus 507 shortcuts. This keeps the graph connected while making random
edges rare.

Run the complete review loop with:

```bash
./scripts/review-optimize-loop.sh <iteration-name>
```

It checks formatting, compilation, tests, and Clippy; runs the deterministic
layout and position-access benchmarks; captures initial, interactive-layout,
and full-detail paused screenshots; and emits benchmark and screenshot
comparisons against the preceding iteration under `artifacts/performance/`.

## Verified result

`small-world-000001` was measured in release mode using the median of three
30-iteration samples:

| Metric | Legacy | Current | Change |
|---|---:|---:|---:|
| Layout total | 563.682 ms | 91.456 ms | 6.16x faster |
| Layout per iteration | 18.789 ms | 3.049 ms | 83.8% lower |
| Interactive edge geometry | 5.807 ms | 4.646 ms | 1.25x faster |

The circular seeded starting position makes the local ring and sparse shortcut
chords visible immediately. Rendering retains interaction-aware edge level of
detail for denser custom graphs, restores full detail when paused, and fits the
settled graph back into the viewport when layout stops.

Compared with the previous 499,361-edge dense stress workload, the small-world
default lowers current layout time per iteration by 38.6% and interactive edge
geometry time by 76.7%. The dense result remains useful as an explicit stress
case, but no longer represents the default example.

## Techniques adopted from Gephi

Gephi's current ForceAtlas2 implementation enables Barnes–Hut at 1,000 nodes,
uses theta 1.2, parallelizes repulsion, assigns node mass from degree, and
controls speed from global swinging and effective traction. GPUG now adopts
those core choices:

- Barnes–Hut quadtree repulsion with theta 1.2;
- parallel per-node repulsion and attraction using Rayon;
- cached compressed-sparse-row adjacency for contention-free attraction;
- degree-based mass and ForceAtlas2-style adaptive speed/jitter control;
- contiguous structure-of-arrays force buffers;
- a single reactive graph entity instead of one entity per node;
- shared immutable edge storage, batched canvas paths, viewport culling, and
  interaction-aware edge level of detail.

Gephi uses a purpose-built OpenGL engine with persistent GPU geometry. GPUG's
GPUI canvas currently rebuilds paths when node positions change, so interactive
edge LOD is the practical substitute until GPUI exposes a suitable custom
GPU-buffer path.

Primary references:

- [Gephi ForceAtlas2 source](https://github.com/gephi/gephi/blob/master/modules/LayoutPlugin/src/main/java/org/gephi/layout/plugin/forceAtlas2/ForceAtlas2.java)
- [Gephi project architecture and OpenGL overview](https://github.com/gephi/gephi)
- [ForceAtlas2 paper](https://doi.org/10.1371/journal.pone.0098679)

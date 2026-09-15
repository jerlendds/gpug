# Local Blade upload-buffer fix

Source: published `blade-util` 0.3.0, MIT licensed,
<https://github.com/kvark/blade>. The published Cargo manifest, README and source
are retained here. GPUI's pinned native renderer depends on this version.

Local change: `BufferBelt::alloc` rounds new capacities geometrically and releases
completed undersized cached buffers when a larger allocation is necessary.
Active/in-flight buffers retain the original synchronization rules. No rendering
geometry, GPU shader, primitive order or graph LOD rule changes.

The former exact-size cache retained a separate allocation for each increasing
vertex count. A GPUG camera sweep with 100,000 nodes/300,000 edges triggered a
kernel OOM kill on 2026-09-15. This patch bounds that retained growth; the benchmark
harness additionally uses its own memory-limited systemd scope.

Validate capacity growth with `cargo test --manifest-path vendor/blade-util/Cargo.toml`.
Desktop memory measurements are required to validate driver behavior as well.

# GPUG developer guide

GPUG separates graph topology, world-space geometry, layout, viewport state,
and rendering. This keeps layout algorithms independent of GPUI and prevents
camera changes from mutating graph geometry.

## Guides

- [Getting started and API overview](api.md)
- [Coordinate systems](coordinates.md)
- [Graph data and generators](data.md)
- [Incremental and batch layouts](layouts.md)
- [Viewport, interaction, and styling](view.md)
- [Performance architecture](performance.md)
- [Migration from the prototype API](migration.md)

The shortest complete setup is:

```rust
use gpug::{Graph, SmallWorld};

let data = SmallWorld::new(10_000)
    .local_neighbors(3)
    .shortcut_probability(0.00001)
    .seed(42)
    .generate();

let graph = Graph::builder()
    .data(data)
    .interactive_layout(true)
    .build(cx)?;
```

`Graph::from_data(data, cx)` is the convenient default when no customization
is needed.

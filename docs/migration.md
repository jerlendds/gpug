# Migrating from the prototype API

## Node coordinates

Old nodes stored `x`, `y`, `pan`, and `zoom` as GPUI state. New nodes store only:

```rust
Node {
    id: NodeId,
    position: WorldPoint,
    size: WorldSize,
    selected: bool,
}
```

Use `graph.world_to_screen` and `screen_to_world` at UI boundaries. Never apply
pan or zoom before invoking a layout.

## Construction

Replace generator-specific construction:

```rust
Graph::new(cx, nodes, edges, k, beta)
```

with:

```rust
Graph::builder().nodes(nodes).edges(edges).build(cx)?
```

`Graph::new` remains as a deprecated compatibility constructor, but `k` and
`beta` are ignored. Configure Watts–Strogatz or any other generator before
constructing the view.

`GpugNode` and `GpugEdge` remain deprecated aliases for `Node` and `Edge`.

## Layouts

The concrete `LayoutWorkspace` is an implementation detail. End-developer code
should use `Layout`, `ForceAtlas2`, `BatchLayoutAdapter`, and `LayoutOptions`.

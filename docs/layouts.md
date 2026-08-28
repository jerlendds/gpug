# Layout API

Layouts operate only on world positions and dense validated topology.

## Incremental layouts

Implement `Layout` for algorithms that can advance incrementally:

```rust
impl Layout for MyLayout {
    fn step(
        &mut self,
        positions: &mut [WorldPoint],
        edges: &[LayoutEdge],
    ) -> LayoutStatus {
        // Update world positions.
        LayoutStatus::Running { energy: 0.5 }
    }
}
```

`initialize` is optional. `ForceAtlas2` is the built-in implementation.
`LayoutOptions::frame_budget` lets GPUG perform as many steps as fit within an
animation-frame budget. `LayoutStatus::Converged` automatically stops the
interactive loop.

```rust
let graph = Graph::builder()
    .data(data)
    .layout(ForceAtlas2::new())
    .interactive_layout(true)
    .build(cx)?;
```

Use `start_layout`, `stop_layout`, `step_layout`, `run_layout`, `set_layout`,
`is_layout_running`, and `layout_frame` for explicit control.

## Batch-only `f64` layouts

Implement `BatchLayout` when an external crate computes the whole layout in one
operation:

```rust
impl BatchLayout for ExternalLayout {
    fn layout(
        &mut self,
        positions: &mut [LayoutPoint],
        edges: &[LayoutEdge],
    ) -> Result<(), String> {
        external_crate::layout_in_place(positions, edges)
            .map_err(|error| error.to_string())
    }
}
```

Wrap it with `BatchLayoutAdapter`, or animate from current to final positions:

```rust
graph.apply_layout_animated(ExternalLayout::default(), 30);
```

## Result fitting

`LayoutFit` is explicit:

- `Preserve`: retain the layout's world coordinates;
- `Center`: translate the result around world origin;
- `Fit`: scale into specified `WorldBounds`, with padding and optional aspect
  ratio preservation.

Configure it with `GraphBuilder::layout_fit` or `LayoutOptions`. Fitting occurs
once after convergence, never while panning, zooming, or stepping.

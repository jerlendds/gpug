# Viewport, interaction, and styling

`Graph` is a GPUI entity that orchestrates validated `GraphData`, a `Layout`, a
`Viewport`, and batched rendering.

## Construction

```rust
let graph = Graph::builder()
    .data(data)
    .layout(ForceAtlas2::default())
    .layout_options(LayoutOptions::default())
    .viewport(Viewport::default())
    .style(GraphStyle::default())
    .interactive_layout(false)
    .fit_on_load()
    .build(cx)?;
```

`Graph::from_data` supplies all defaults.

## View operations

- `viewport`, `set_viewport`, `set_pan`, `pan_by`
- `world_to_screen`, `screen_to_world`
- `set_zoom`
- `center_on(node_id, screen_center)`
- `fit_to_view(screen_size, padding)`

Scroll-wheel and trackpad zoom is proportional, cursor anchored, and ranges
from `0.001x` to `256x`. The limits are exposed as `Viewport::MIN_ZOOM` and
`Viewport::MAX_ZOOM`. Clicking selects a node; Shift-click adds to the
selection. Hit testing is performed in screen space.

Click-dragging empty canvas space pans the viewport. The cursor is an open hand
over pannable canvas space and a closed hand during a pan. Nodes and edges use
the default arrow cursor and do not initiate canvas panning, so clicking a node
continues to select it without moving the viewport.

## Styling

`GraphStyle` configures background, node, edge, and selection colors; world
node radius; screen-pixel edge width and hit radius; and the interactive edge
budget. Use `GraphBuilder::style`, `Graph::style`, and `Graph::set_style`.
`GraphRenderer` owns that policy independently of data and layout. Supply one
with `GraphBuilder::renderer` or inspect it with `Graph::renderer`.

When a dense graph is actively laying out, GPUG samples edges down to
`interactive_edge_budget`. Pausing restores every edge. Layout calculations
always use the complete topology.

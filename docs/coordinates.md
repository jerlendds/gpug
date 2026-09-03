# Coordinate systems

GPUG uses three intentionally distinct coordinate spaces.

| Space | Type | Precision | Purpose |
|---|---|---:|---|
| Layout | `LayoutPoint` | `f64` | Boundary for external layout crates |
| World | `WorldPoint`, `WorldSize`, `WorldBounds` | `f32` | Authoritative graph geometry |
| Screen | GPUI `Point<Pixels>` | GPUI pixels | Painting and pointer events |

`Graph::world_to_screen` and `Graph::screen_to_world` use GPUI window
coordinates. GPUG accounts for the graph component's laid-out origin, so the
same pointer position can be passed directly when the graph is nested beside a
sidebar, header, or other content. `Viewport` transforms remain local to the
graph pane.

The graph stores only world positions. Pan and zoom live in `Viewport` and are
never passed to a layout engine:

```rust
let screen = graph.world_to_screen(node.position);
let world = graph.screen_to_world(mouse_event.position);
```

The transform is:

```text
screen = pan + world * zoom
world  = (screen - pan) / zoom
```

`Viewport::zoom_about` updates pan while zooming so the world point under the
cursor remains fixed. `Graph::set_zoom`, `Graph::center_on`, and
`Graph::fit_to_view` provide common camera operations.

## External layouts

External batch layouts may use arbitrary `f64` units. `BatchLayoutAdapter`
converts `WorldPoint` to `LayoutPoint` before the call and converts back once.
Camera state is not involved. Use `LayoutFit` to center or normalize the final
result explicitly; do not fit every animation frame.

Node sizes use `WorldSize` and therefore scale with zoom. Edge width and hit
radius are configured in screen pixels so they remain usable at every zoom.

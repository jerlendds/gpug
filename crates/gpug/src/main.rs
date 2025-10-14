use gpui::*;
use gpui::{
    canvas, div, Application, Context, IntoElement, ParentElement, Render, Styled, Window,
    WindowOptions,
};

impl Render for Graph {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Batched edges canvas: draw all edges in a single paint pass
        let edge_pairs = self.edge_pairs.clone();
        let nodes_for_edges = self.nodes.clone();
        let zoom = self.zoom;
        let pan = self.pan;
        let graph_entity = cx.entity();
        let graph_id = graph_entity.entity_id();
        let edges_canvas = canvas(
            move |_bounds, _window, _cx| (),
            move |_bounds, _state, window, cx| {
                let mut path = gpui::Path::new(point(px(0.0), px(0.0)));
                let thickness = (0.5f32 * zoom).max(0.5);
                for &(i, j) in &edge_pairs {
                    if i >= nodes_for_edges.len() || j >= nodes_for_edges.len() {
                        continue;
                    }
                    let (x1, y1) =
                        cx.read_entity(&nodes_for_edges[i], |n, _| (n.x + px(8.0), n.y + px(8.0)));
                    let (x2, y2) =
                        cx.read_entity(&nodes_for_edges[j], |n, _| (n.x + px(8.0), n.y + px(8.0)));

                    let p1 = point(pan.x + x1 * zoom, pan.y + y1 * zoom);
                    let p2 = point(pan.x + x2 * zoom, pan.y + y2 * zoom);
                    let dir = point(p2.x - p1.x, p2.y - p1.y);
                    let len = dir.magnitude() as f32;
                    if len <= 0.0001 {
                        continue;
                    }
                    let half_thickness: f32 = thickness as f32;
                    let normal = point(-dir.y, dir.x) * (half_thickness / len);

                    let p1a = point(p1.x + normal.x, p1.y + normal.y);
                    let p1b = point(p1.x - normal.x, p1.y - normal.y);
                    let p2a = point(p2.x + normal.x, p2.y + normal.y);
                    let p2b = point(p2.x - normal.x, p2.y - normal.y);

                    let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                    path.push_triangle((p1a, p1b, p2a), st);
                    path.push_triangle((p2a, p1b, p2b), st);
                }
                window.paint_path(path, rgb(0x323232));
            },
        )
        .absolute()
        .size_full();

        // Node entities render above edges
        let graph_canvas = div()
            .size_full()
            .child(edges_canvas)
            .children(self.nodes.iter().cloned());

        // Simulation canvas: runs a physics step per frame when playing
        let graph_handle = graph_entity.clone();
        let nodes_for_sim = self.nodes.clone();
        let edge_pairs = self.edge_pairs.clone();
        let sim_canvas = canvas(
            move |_bounds, _window, _cx| (),
            move |_bounds, _state, _window, cx| {
                let playing = cx.read_entity(&graph_handle, |g: &Graph, _| g.playing);
                if !playing {
                    return;
                }
                let n = nodes_for_sim.len();
                if n == 0 {
                    return;
                }

                // Read positions
                let mut xs: Vec<f32> = Vec::with_capacity(n);
                let mut ys: Vec<f32> = Vec::with_capacity(n);
                for ent in &nodes_for_sim {
                    let (x, y) = cx.read_entity(ent, |nd, _| (nd.x, nd.y));
                    xs.push((x / px(1.0)) as f32);
                    ys.push((y / px(1.0)) as f32);
                }

                let mut fx = vec![0.0f32; n];
                let mut fy = vec![0.0f32; n];

                // Force parameters (tune for stability/perf)
                let repulsion = 120.0f32; // lower repulsion reduces oscillation
                let attraction = 0.03f32; // stronger springs for faster settling
                let gravity = 0.006f32; // pull toward center
                let damping = 0.85f32; // velocity damping
                let dt = 0.5f32; // larger step, clamped below
                let max_disp = 5.0f32; // cap displacement per step
                let center_x = 800.0f32;
                let center_y = 200.0f32;

                // Spatial grid for approximate repulsion
                use std::collections::HashMap;
                let cell = 100.0f32;
                let mut bins: HashMap<(i32, i32), Vec<usize>> = HashMap::with_capacity(n * 2);
                for i in 0..n {
                    let gx = (xs[i] / cell).floor() as i32;
                    let gy = (ys[i] / cell).floor() as i32;
                    bins.entry((gx, gy)).or_default().push(i);
                }
                let neighbors = [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (0, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ];
                for i in 0..n {
                    let gx = (xs[i] / cell).floor() as i32;
                    let gy = (ys[i] / cell).floor() as i32;
                    for (dxg, dyg) in neighbors {
                        if let Some(v) = bins.get(&(gx + dxg, gy + dyg)) {
                            for &j in v {
                                if j <= i {
                                    continue;
                                }
                                let dx = xs[j] - xs[i];
                                let dy = ys[j] - ys[i];
                                let mut d2 = dx * dx + dy * dy + 0.01;
                                let inv = 1.0 / d2;
                                let fx_ij = repulsion * dx * inv;
                                let fy_ij = repulsion * dy * inv;
                                fx[i] -= fx_ij;
                                fy[i] -= fy_ij;
                                fx[j] += fx_ij;
                                fy[j] += fy_ij;
                            }
                        }
                    }
                }

                // Attraction along edges
                for &(i, j) in &edge_pairs {
                    if i >= n || j >= n {
                        continue;
                    }
                    let dx = xs[j] - xs[i];
                    let dy = ys[j] - ys[i];
                    let fx_e = attraction * dx;
                    let fy_e = attraction * dy;
                    fx[i] += fx_e;
                    fy[i] += fy_e;
                    fx[j] -= fx_e;
                    fy[j] -= fy_e;
                }

                // Gravity towards center
                for i in 0..n {
                    fx[i] += gravity * (center_x - xs[i]);
                    fy[i] += gravity * (center_y - ys[i]);
                }

                // Integrate and clamp small step
                for i in 0..n {
                    let mut dx = fx[i] * dt;
                    let mut dy = fy[i] * dt;
                    dx *= damping;
                    dy *= damping;
                    let disp2 = dx * dx + dy * dy;
                    if disp2 > max_disp * max_disp {
                        let s = max_disp / disp2.sqrt();
                        dx *= s;
                        dy *= s;
                    }
                    xs[i] += dx;
                    ys[i] += dy;
                }

                // Write back
                for i in 0..n {
                    let nx = px(xs[i] as f32);
                    let ny = px(ys[i] as f32);
                    let ent = nodes_for_sim[i].clone();
                    cx.update_entity(&ent, move |node, _| {
                        node.x = nx;
                        node.y = ny;
                    });
                }
                // Schedule next frame: nudge state and request another pass
                cx.update_entity(&graph_handle, |g: &mut Graph, _| {
                    g.sim_tick = g.sim_tick.wrapping_add(1);
                });
                cx.notify(graph_handle.entity_id());
            },
        )
        .absolute()
        .size_full();

        let play_button = div()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .size(px(28.0))
            .rounded_full()
            .bg(if self.playing {
                rgb(0x4CAF50)
            } else {
                rgb(0xeeeeee)
            })
            .border(px(1.0))
            .border_color(rgb(0xcccccc))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener({
                    move |this, _e: &gpui::MouseDownEvent, _w, cx| {
                        this.playing = !this.playing;
                        cx.notify();
                    }
                }),
            );

        div()
            .size_full()
            .bg(rgb(0xffffff))
            .child(sim_canvas)
            // Clicking selects node under cursor; shift adds to selection; clicking empty space clears
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, _w, cx| {
                    let cursor = e.position;
                    let mut hit_index: Option<usize> = None;
                    for (i, n) in this.nodes.iter().enumerate() {
                        let (nx, ny) = cx.read_entity(n, |node, _| (node.x, node.y));
                        let left = this.pan.x + nx * this.zoom;
                        let top = this.pan.y + ny * this.zoom;
                        let size = px(16.0) * this.zoom;
                        if cursor.x >= left
                            && cursor.x <= left + size
                            && cursor.y >= top
                            && cursor.y <= top + size
                        {
                            hit_index = Some(i);
                            break;
                        }
                    }

                    match hit_index {
                        Some(i) => {
                            let shift = e.modifiers.shift;
                            if !shift {
                                for n in &this.nodes {
                                    cx.update_entity(n, |node, _| node.selected = false);
                                }
                            }
                            let target = this.nodes[i].clone();
                            cx.update_entity(&target, |_node, _| {}); // ensure entity exists
                            cx.update_entity(&target, |node, _| {
                                node.selected = true;
                            });
                        }
                        None => {
                            for n in &this.nodes {
                                cx.update_entity(n, |node, _| node.selected = false);
                            }
                        }
                    }
                    // selection updates above trigger re-render
                }),
            )
            .on_scroll_wheel(cx.listener({
                let graph_id = graph_id;
                move |this, event: &gpui::ScrollWheelEvent, _window, cx| {
                    let delta_px = event.delta.pixel_delta(px(16.0));
                    let dy = delta_px.y;

                    if dy != px(0.0) {
                        let factor = if dy > px(0.0) { 1.1 } else { 0.9 };
                        let old_zoom = this.zoom;
                        let new_zoom = (old_zoom * factor).clamp(0.25, 4.0);

                        // Zoom toward cursor position by adjusting pan
                        let s = event.position;
                        let world_x = (s.x - this.pan.x) / old_zoom;
                        let world_y = (s.y - this.pan.y) / old_zoom;
                        this.pan = point(s.x - world_x * new_zoom, s.y - world_y * new_zoom);

                        this.zoom = new_zoom;
                        for n in &this.nodes {
                            let pan = this.pan;
                            let zoom = this.zoom;
                            cx.update_entity(n, move |node, _| {
                                node.zoom = zoom;
                                node.pan = pan;
                            });
                        }
                        // ensure the graph re-renders so shared canvases reflect new zoom/pan
                        cx.notify();
                    }
                }
            }))
            .child(graph_canvas)
            .child(play_button)
    }
}

// Simple draggable node
pub struct GpugNode {
    id: u64,
    x: Pixels,
    y: Pixels,
    // Offset from the node's origin to the cursor at drag start
    drag_offset: Option<Point<Pixels>>,
    zoom: f32,
    pan: Point<Pixels>,
    selected: bool,
}

impl Render for GpugNode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let node = div()
            .size(px(16.0 * self.zoom))
            .rounded_full()
            .bg(rgb(0x000000))
            .cursor_move()
            .id(("node", self.id as usize))
            // Start a drag with this node's id as payload; lets listeners filter events
            .on_drag(self.id, |_id: &u64, _offset, _window, cx| {
                cx.new(|_| DragPreview)
            })
            // Update position while dragging only if this node is the dragged one
            .on_drag_move::<u64>(
                cx.listener(|this, event: &DragMoveEvent<u64>, _window, cx| {
                    if *event.drag(cx) != this.id {
                        return;
                    }
                    // Record the initial cursor offset inside the node on first move
                    if this.drag_offset.is_none() {
                        let offset = point(
                            (event.event.position.x - event.bounds.left()) / this.zoom,
                            (event.event.position.y - event.bounds.top()) / this.zoom,
                        );
                        this.drag_offset = Some(offset);
                    }

                    if let Some(offset) = this.drag_offset {
                        let new_origin = point(
                            (event.event.position.x - this.pan.x) / this.zoom - offset.x,
                            (event.event.position.y - this.pan.y) / this.zoom - offset.y,
                        );
                        this.x = new_origin.x;
                        this.y = new_origin.y;
                        // position changes trigger re-render
                    }
                }),
            )
            .on_drop(cx.listener(|this, dragged_id: &u64, _window, cx| {
                if *dragged_id == this.id {
                    this.drag_offset = None;
                }
            }));

        if self.selected {
            // Wrap the dot with a positioned container that provides
            // a fixed 10px gap between the border and the black node
            div()
                .absolute()
                .left(self.pan.x + self.x * self.zoom - px(10.0))
                .top(self.pan.y + self.y * self.zoom - px(10.0))
                .p(px(10.0))
                .border(px(4.0))
                .rounded_full()
                .border_color(rgb(0x1E90FF))
                .child(node)
        } else {
            // Unselected: position with a wrapper so both branches return the same type
            div()
                .absolute()
                .left(self.pan.x + self.x * self.zoom)
                .top(self.pan.y + self.y * self.zoom)
                .child(node)
        }
    }
}

// Minimal drag preview view to satisfy on_drag constructor
struct DragPreview;
impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Invisible 1x1 element as drag ghost
        div().size(px(1.0)).bg(rgb(0xffffff)).opacity(0.0)
    }
}

type GraphNodes = Vec<Entity<GpugNode>>;
type GraphEdges = Vec<Entity<GpugEdge>>;
// Root view that hosts nodes
struct Graph {
    nodes: GraphNodes,
    edges: GraphEdges,
    edge_pairs: Vec<(usize, usize)>,
    sim_tick: u64,
    zoom: f32,
    pan: Point<Pixels>,
    playing: bool,
}

impl Graph {
    fn new(cx: &mut App, total_nodes: usize, random_link_chance: f32) -> Self {
        // Create a random graph: N nodes, edges with probability p
        let nodes: Vec<Entity<GpugNode>> = generate_nodes(cx, total_nodes);
        let (edges, edge_pairs) = randomly_link_nodes_with_pairs(cx, &nodes, random_link_chance);
        Self {
            nodes,
            edges,
            edge_pairs,
            sim_tick: 0,
            zoom: 1.0,
            pan: point(px(0.0), px(0.0)),
            playing: false,
        }
    }
}

// Edge connecting two nodes by drawing a straight path between their centers
struct GpugEdge {
    from: Entity<GpugNode>,
    to: Entity<GpugNode>,
    zoom: f32,
    pan: Point<Pixels>,
}

impl GpugEdge {
    fn new(from: Entity<GpugNode>, to: Entity<GpugNode>) -> Self {
        Self {
            from,
            to,
            zoom: 1.0,
            pan: point(px(0.0), px(0.0)),
        }
    }
}

impl Render for GpugEdge {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read node positions (centers) at render time
        let (x1, y1) = cx.read_entity(&self.from, |n, _| (n.x + px(8.0), n.y + px(8.0)));
        let (x2, y2) = cx.read_entity(&self.to, |n, _| (n.x + px(8.0), n.y + px(8.0)));

        let edge_zoom = self.zoom;
        let edge_pan = self.pan;
        let p1 = point(edge_pan.x + x1 * edge_zoom, edge_pan.y + y1 * edge_zoom);
        let p2 = point(edge_pan.x + x2 * edge_zoom, edge_pan.y + y2 * edge_zoom);

        // Use gpui's canvas element to paint a thin rectangle between points
        canvas(
            move |_bounds, _window, _cx| (p1, p2),
            move |_bounds, (p1, p2), window, _cx| {
                let dir = point(p2.x - p1.x, p2.y - p1.y);
                let len = dir.magnitude() as f32;
                if len <= 0.0001 {
                    return;
                }
                let half_thickness: f32 = 0.5 * edge_zoom;
                let normal = point(-dir.y, dir.x) * (half_thickness / len);

                let p1a = point(p1.x + normal.x, p1.y + normal.y);
                let p1b = point(p1.x - normal.x, p1.y - normal.y);
                let p2a = point(p2.x + normal.x, p2.y + normal.y);
                let p2b = point(p2.x - normal.x, p2.y - normal.y);

                let mut path = gpui::Path::new(p1a);
                // avoids tapered alpha in path shader
                let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                path.push_triangle((p1a, p1b, p2a), st);
                path.push_triangle((p2a, p1b, p2b), st);

                window.paint_path(path, rgb(0x323232));
            },
        )
        .absolute()
        .size_full()
    }
}

// Simple xorshift-based PRNG to avoid external dependencies
fn rng_next(seed: &mut u64) -> u64 {
    // Xorshift64*
    let mut x = *seed;
    if x == 0 {
        x = 0x9E3779B97F4A7C15; // avoid zero state
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    x
}

fn rand_f32(seed: &mut u64) -> f32 {
    // Convert upper bits to [0,1)
    let r = rng_next(seed);
    let v = (r >> 11) as u32; // 53 bits -> 42 bits -> fit in f32 mantissa
    (v as f32) / (u32::MAX as f32)
}

// Generate n nodes with random positions within a region
fn generate_nodes(cx: &mut App, n: usize) -> Vec<Entity<GpugNode>> {
    let mut seed: u64 = 0xCAFEBABEDEADBEEF;
    let mut nodes: Vec<Entity<GpugNode>> = Vec::with_capacity(n);

    // Scatter in a reasonable viewport box
    let left = 50.0f32;
    let top = 50.0f32;
    let width = 1200.0f32;
    let height = 800.0f32;

    for i in 0..n {
        let rx = rand_f32(&mut seed);
        let ry = rand_f32(&mut seed);
        let x = px(left + rx * width);
        let y = px(top + ry * height);
        nodes.push(cx.new(|_| GpugNode {
            id: (i as u64) + 1,
            x,
            y,
            drag_offset: None,
            zoom: 1.0,
            pan: point(px(0.0), px(0.0)),
            selected: false,
        }));
    }
    nodes
}

// Randomly link nodes pairwise with a given probability
fn randomly_link_nodes_with_pairs(
    cx: &mut App,
    nodes: &[Entity<GpugNode>],
    probability: f32,
) -> (Vec<Entity<GpugEdge>>, Vec<(usize, usize)>) {
    let mut seed: u64 = 0x1234_5678_ABCD_EF01;
    let mut edges: Vec<Entity<GpugEdge>> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let n = nodes.len();
    if n < 2 {
        return (edges, pairs);
    }
    let p = probability.clamp(0.0, 1.0);
    for i in 0..n {
        for j in (i + 1)..n {
            if rand_f32(&mut seed) < p {
                let a = nodes[i].clone();
                let b = nodes[j].clone();
                edges.push(cx.new(|_| GpugEdge::new(a, b)));
                pairs.push((i, j));
            }
        }
    }
    (edges, pairs)
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| Graph::new(cx, 100, 0.04))
        })
        .unwrap();
    });
}

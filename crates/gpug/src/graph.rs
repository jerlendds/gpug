use gpui::*;
use gpui::{canvas, div, Context, IntoElement, ParentElement, Render, Styled, Window};

use crate::edge::GpugEdge;
use crate::generators::watts_strogatz::generate_watts_strogatz_graph;
use crate::node::GpugNode;

pub struct Graph {
    pub nodes: Vec<Entity<GpugNode>>,
    pub edges: Vec<GpugEdge>,
    pub k: usize,
    pub beta: f32,
    pub sim_tick: u64,
    pub zoom: f32,
    pub pan: Point<Pixels>,
    pub playing: bool,
}

impl Graph {
    pub fn new(
        cx: &mut App,
        nodes: Vec<GpugNode>,
        edges: Vec<GpugEdge>,
        k: usize,
        beta: f32,
    ) -> Self {
        let zoom = 1.0;
        let pan = point(px(0.0), px(0.0));
        let mut node_entities: Vec<Entity<GpugNode>> = Vec::with_capacity(nodes.len());

        for mut node in nodes {
            node.zoom = zoom;
            node.pan = pan;
            node_entities.push(cx.new(|_| node));
        }

        Self {
            nodes: node_entities,
            edges,
            k,
            beta,
            sim_tick: 0,
            zoom,
            pan,
            playing: false,
        }
    }

    fn max_k(&self) -> usize {
        self.nodes.len().saturating_sub(1).saturating_div(2).max(1)
    }

    fn adjust_k(&mut self, delta: isize, cx: &mut Context<Self>) {
        let node_count = self.nodes.len();
        if node_count < 2 {
            return;
        }
        let max_k = self.max_k() as isize;
        let mut new_k = self.k as isize + delta;
        if max_k < 1 {
            return;
        }
        if new_k < 1 {
            new_k = 1;
        }
        if new_k > max_k {
            new_k = max_k;
        }
        if new_k as usize == self.k {
            return;
        }
        self.k = new_k as usize;
        self.edges = generate_watts_strogatz_graph(node_count, self.k, self.beta);
        cx.notify();
    }

    fn adjust_beta(&mut self, delta: f32, cx: &mut Context<Self>) {
        let new_beta = (self.beta + delta).clamp(0.0, 1.0);
        if (new_beta - self.beta).abs() < 1e-4 {
            return;
        }
        self.beta = new_beta;
        let node_count = self.nodes.len();
        if node_count < 2 {
            return;
        }
        self.edges = generate_watts_strogatz_graph(node_count, self.k, self.beta);
        cx.notify();
    }
}

fn parameter_button<F>(label: &str, cx: &mut Context<Graph>, on_press: F) -> Div
where
    F: Fn(&mut Graph, &mut Context<Graph>) + 'static,
{
    div()
        .child(label.to_string())
        .p(px(4.0))
        .bg(rgb(0xf0f0f0))
        .border(px(1.0))
        .border_color(rgb(0xcccccc))
        .rounded(px(4.0))
        .cursor_pointer()
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, _event: &gpui::MouseDownEvent, _window, cx| {
                on_press(this, cx);
            }),
        )
}

impl Render for Graph {
    fn render(&mut self, _window: &mut Window, graph_cx: &mut Context<Self>) -> impl IntoElement {
        // Batched edges canvas: draw all edges in a single paint pass
        let zoom = self.zoom;
        let pan = self.pan;
        let nodes = self.nodes.clone();
        let edges = self.edges.clone();
        let graph_entity = graph_cx.entity();
        let edges_canvas = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _state, window, cx| {
                let mut path = gpui::Path::new(point(px(0.0), px(0.0)));
                let thickness = (0.5f32 * zoom).max(0.5);
                for edge in &edges {
                    let i = edge.source;
                    let j = edge.target;
                    if i >= nodes.len() || j >= nodes.len() {
                        continue;
                    }
                    let (x1, y1) = cx.read_entity(&nodes[i], |n, _| (n.x + px(8.0), n.y + px(8.0)));
                    let (x2, y2) = cx.read_entity(&nodes[j], |n, _| (n.x + px(8.0), n.y + px(8.0)));

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

        let max_k = self.max_k();
        let controls_panel = {
            let decrease_k = parameter_button("-", graph_cx, |this, cx| {
                this.adjust_k(-1, cx);
            });
            let increase_k = parameter_button("+", graph_cx, |this, cx| {
                this.adjust_k(1, cx);
            });
            let beta_step = 0.05f32;
            let decrease_beta = parameter_button("-", graph_cx, move |this, cx| {
                this.adjust_beta(-beta_step, cx);
            });
            let increase_beta = parameter_button("+", graph_cx, move |this, cx| {
                this.adjust_beta(beta_step, cx);
            });

            div()
                .absolute()
                .top(px(8.0))
                .left(px(8.0))
                .bg(rgb(0xf7f7f7))
                .border(px(1.0))
                .border_color(rgb(0xcccccc))
                .rounded(px(6.0))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(format!("k: {} / {}", self.k, max_k))
                        .child(decrease_k)
                        .child(increase_k),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(format!("beta: {:.2}", self.beta))
                        .child(decrease_beta)
                        .child(increase_beta),
                )
        };

        // Simulation canvas: runs a physics step per frame when playing
        let graph_handle = graph_entity.clone();
        let nodes_for_sim = self.nodes.clone();
        let edges = self.edges.clone();
        let sim_canvas = canvas(
            move |_bounds, _window, _cx| (),
            move |_bounds, _state, window, cx| {
                let playing = cx.read_entity(&graph_handle, |g: &Graph, _| g.playing);
                if !playing {
                    return;
                }
                let n = nodes_for_sim.len();
                if n == 0 {
                    return;
                }

                window.request_animation_frame();

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
                                let d2 = dx * dx + dy * dy + 0.01;
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
                for edge in &edges {
                    let i = edge.source;
                    let j = edge.target;
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
                // Bookkeep a tick so any observers can react and mark the graph dirty
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
                graph_cx.listener({
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
                graph_cx.listener(|this, e: &gpui::MouseDownEvent, _w, cx| {
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
                }),
            )
            .on_scroll_wheel(graph_cx.listener({
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
            .child(controls_panel)
            .child(play_button)
    }
}

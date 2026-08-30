use gpug::{
    Edge, Graph, GraphData, GraphRenderer, GraphStyle, Node, NodeAppearance, NodeShape, Position,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, px, rgb, App, AppContext, Application, Context, Entity, IntoElement,
    ParentElement, Render, Styled, Window, WindowOptions,
};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const NODE_COUNT: usize = 1_000;
const COLUMNS: usize = 20;
const ROWS: usize = NODE_COUNT / COLUMNS;
const NODE_SIZE: WorldSize = WorldSize::new(11.0, 7.0);
const SAMPLE_CAPACITY: usize = 120;

/// Drives the camera through a zoom sweep so one run measures the cost of
/// every scale, not just the one the graph happened to open at. Zoom is the
/// input most scene state is derived from, so a sweep is what exercises the
/// paths a fixed camera never reaches: culling, the node level-of-detail
/// ladder, and the promotion of small nodes into full element content.
///
/// It re-centres on the graph every frame. While the layout runs the graph
/// drifts and expands, and a camera that only tracked zoom would quietly end
/// up pointed at empty space - measuring a blank window at a very good frame
/// rate.
struct ZoomSweep {
    started: Instant,
    /// When set, hold this scale instead of sweeping. Used to measure or
    /// inspect one zoom level in isolation.
    fixed: Option<f32>,
}

impl ZoomSweep {
    /// Scale relative to the zoom that fits the graph, swept from well out to
    /// well in and back.
    fn scale(&self) -> f32 {
        if let Some(fixed) = self.fixed {
            return fixed;
        }
        let phase = self.started.elapsed().as_secs_f32() * 0.6;
        9.0f32.powf(phase.sin())
    }
}

struct StressView {
    graph: Entity<Graph>,
    sweep: Option<ZoomSweep>,
    previous_frame: Option<Instant>,
    samples: VecDeque<f32>,
    last_publish: Instant,
    fps: f32,
    p95_ms: f32,
}

impl StressView {
    fn new(graph: Entity<Graph>) -> Self {
        Self {
            graph,
            sweep: None,
            previous_frame: None,
            samples: VecDeque::with_capacity(SAMPLE_CAPACITY),
            last_publish: Instant::now(),
            fps: 0.0,
            p95_ms: 0.0,
        }
    }

    fn record_frame(&mut self) -> bool {
        let now = Instant::now();
        if let Some(previous) = self.previous_frame.replace(now) {
            if self.samples.len() == SAMPLE_CAPACITY {
                self.samples.pop_front();
            }
            self.samples
                .push_back(now.duration_since(previous).as_secs_f32() * 1_000.0);
        }
        if now.duration_since(self.last_publish) < Duration::from_millis(250)
            || self.samples.is_empty()
        {
            return false;
        }
        self.last_publish = now;
        let average = self.samples.iter().sum::<f32>() / self.samples.len() as f32;
        self.fps = 1_000.0 / average.max(0.001);
        let mut sorted = self.samples.iter().copied().collect::<Vec<_>>();
        sorted.sort_by(f32::total_cmp);
        self.p95_ms = sorted[((sorted.len() - 1) as f32 * 0.95).round() as usize];
        true
    }
}

impl Render for StressView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The camera is driven here rather than from the ticker's paint
        // callback. A write made during paint lands after the graph has
        // already rendered, so the change is never picked up and the view
        // silently stays where it was.
        if let Some(sweep) = &self.sweep {
            let scale = sweep.scale();
            let size = window.viewport_size();
            self.graph.update(cx, |graph, cx| {
                let Some(bounds) = graph.content_bounds() else {
                    return;
                };
                // Re-derived every frame: a running layout expands the graph,
                // and a camera that framed only the starting positions would
                // end up pointed at empty space.
                let fitted = (size.width / px(1.0) / bounds.size.width.max(1.0))
                    .min(size.height / px(1.0) / bounds.size.height.max(1.0));
                let center = WorldPoint::new(
                    bounds.origin.x + bounds.size.width * 0.5,
                    bounds.origin.y + bounds.size.height * 0.5,
                );
                graph.set_center(center, size, fitted * scale);
                cx.notify();
            });
        }

        let view = cx.entity();
        let sweeping = self.sweep.is_some();
        let ticker = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                cx.update_entity(&view, |view, cx| {
                    // A sweep needs a re-render every frame; otherwise only
                    // publishing the frame meter does.
                    if view.record_frame() || sweeping {
                        cx.notify();
                    }
                });
            },
        )
        .absolute()
        .size(px(1.0));
        let label = if self.samples.is_empty() {
            "FPS: measuring…".into()
        } else {
            format!("FPS: {:.1}\np95: {:.2} ms", self.fps, self.p95_ms)
        };
        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(ticker)
            .child(
                div()
                    .absolute()
                    .right(px(8.0))
                    .bottom(px(8.0))
                    .bg(rgb(0xf7f7f7))
                    .border(px(1.0))
                    .border_color(rgb(0xcccccc))
                    .rounded(px(6.0))
                    .p(px(8.0))
                    .child(label),
            )
    }
}

/// The cheap description of a node: what the graph paints when the node is too
/// small on screen to be worth an element tree. It mirrors the registered
/// content's shell exactly, so crossing the level-of-detail threshold changes
/// how the node is drawn but not how it looks.
fn stress_node(node: &Node, _: f32, _: &GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: NODE_SIZE.width * 0.5,
        shape: NodeShape::Rect {
            corner_radius_world: 1.6,
            border_color: if node.selected { 0xff5eae } else { 0xd4d4d8 },
            border_width_pixels: 1.0,
        },
    }
}

fn stress_data() -> GraphData {
    let mut nodes = Vec::with_capacity(NODE_COUNT);
    let mut edges = Vec::with_capacity(NODE_COUNT);
    for index in 0..NODE_COUNT {
        let column = index / ROWS;
        let row = index % ROWS;
        let id = index as u64 + 1;
        let mut node = Node::new(id, WorldPoint::new(column as f32 * 18.0, row as f32 * 15.0))
            .with_size(NODE_SIZE)
            .with_type("stress");
        node.metadata
            .insert("caption".into(), format!("Node\n{id}"));
        nodes.push(node);

        // One cycle gives every node exactly one incoming and outgoing edge.
        let target = (index + 1) % NODE_COUNT + 1;
        edges.push(Edge::new_with_id(id, target as u64, id));
    }
    GraphData::new(nodes, edges)
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Stress Test".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("stress", stress_node);
                renderer.register_cached_node_content("stress", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(1.6 * zoom))
                        .border(px(1.0))
                        .border_color(rgb(if node.selected { 0xff5eae } else { 0xd4d4d8 }))
                        .bg(rgb(0xffffff))
                        .text_size(px(2.0 * zoom))
                        .child(node.metadata["caption"].clone())
                        .into_any_element()
                });
                let animate = std::env::var("GPUG_ANIMATE").is_ok_and(|value| value != "0");
                let graph = cx.new(|cx| {
                    let mut graph = Graph::builder()
                        .data(stress_data())
                        .renderer(renderer)
                        .handle_positions(Position::Top, Position::Bottom)
                        .show_handles(true)
                        .only_render_visible_elements(
                            !std::env::var("GPUG_CULL").is_ok_and(|value| value == "0"),
                        )
                        .fit_on_load()
                        .build(cx)
                        .unwrap();
                    if animate {
                        graph.start_layout();
                    }
                    graph
                });
                cx.new(|_| {
                    let mut view = StressView::new(graph);
                    let fixed = std::env::var("GPUG_ZOOM_SCALE")
                        .ok()
                        .and_then(|value| value.parse().ok());
                    if fixed.is_some()
                        || std::env::var("GPUG_ZOOM_SWEEP").is_ok_and(|value| value != "0")
                    {
                        view.sweep = Some(ZoomSweep {
                            started: Instant::now(),
                            fixed,
                        });
                    }
                    view
                })
            },
        )
        .unwrap();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_node_has_one_incoming_and_outgoing_edge() {
        let data = stress_data();
        assert_eq!(
            (data.nodes.len(), data.edges.len()),
            (NODE_COUNT, NODE_COUNT)
        );
        for node in &data.nodes {
            assert_eq!(
                data.edges
                    .iter()
                    .filter(|edge| edge.source == node.id)
                    .count(),
                1
            );
            assert_eq!(
                data.edges
                    .iter()
                    .filter(|edge| edge.target == node.id)
                    .count(),
                1
            );
            assert!(node.draggable);
        }
    }
}

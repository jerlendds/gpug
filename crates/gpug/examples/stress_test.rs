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

struct StressView {
    graph: Entity<Graph>,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let ticker = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                cx.update_entity(&view, |view, cx| {
                    if view.record_frame() {
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

fn stress_node(_: &Node, _: f32, _: &GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: NODE_SIZE.width * 0.5,
        shape: NodeShape::None,
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
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(stress_data())
                        .renderer(renderer)
                        .handle_positions(Position::Top, Position::Bottom)
                        .show_handles(true)
                        .only_render_visible_elements(true)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| StressView::new(graph))
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

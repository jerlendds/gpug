#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpug::Graph;
use gpug::{
    Edge, EdgeMarker, GraphData, GraphEvent, GraphRenderer, Node, NodeAppearance, NodeShape,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, prelude::*, px, rgb, App, AppContext, Application, Context, Entity, IntoElement,
    Render, Window, WindowOptions,
};

fn square_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0x4263eb,
        radius_pixels: 8.0,
        shape: NodeShape::Square,
    }
}

fn diamond_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0x4263eb,
        radius_pixels: 8.0,
        shape: NodeShape::Diamond,
    }
}

/// Reacts to graph events on behalf of an example. The harness drains events
/// once per frame and hands each one to the example before logging it.
pub type ExampleEventHandler = fn(&mut Graph, &GraphEvent, &mut Context<Graph>);

/// Declarative input used by the focused examples in this directory. Keeping
/// window setup here makes every example file about the behavior it presents.
pub struct CatalogExample {
    pub title: &'static str,
    pub summary: &'static str,
    pub node_types: &'static [&'static str],
    pub edge_types: &'static [&'static str],
    pub node_count: usize,
    pub show_handles: bool,
    pub on_event: Option<ExampleEventHandler>,
}

impl CatalogExample {
    pub const fn new(title: &'static str, summary: &'static str) -> Self {
        Self {
            title,
            summary,
            node_types: &["default"],
            edge_types: &["default"],
            node_count: 5,
            show_handles: false,
            on_event: None,
        }
    }

    pub const fn node_types(mut self, types: &'static [&'static str]) -> Self {
        self.node_types = types;
        self
    }
    pub const fn edge_types(mut self, types: &'static [&'static str]) -> Self {
        self.edge_types = types;
        self
    }
    pub const fn node_count(mut self, count: usize) -> Self {
        self.node_count = count;
        self
    }
    pub const fn show_handles(mut self, visible: bool) -> Self {
        self.show_handles = visible;
        self
    }
    pub const fn on_event(mut self, handler: ExampleEventHandler) -> Self {
        self.on_event = Some(handler);
        self
    }
}

pub fn run_catalog_example(example: CatalogExample) {
    Application::new().run(move |cx: &mut App| {
        let options = WindowOptions {
            app_id: Some(format!("GPUG — {}", example.title)),
            ..Default::default()
        };
        cx.open_window(options, move |_, cx| {
            let graph = cx.new(|cx| {
                let count = example.node_count.max(2);
                let mut nodes = Vec::with_capacity(count);
                for index in 0..count {
                    let column = index % 5;
                    let row = index / 5;
                    let mut node = Node::new(
                        index as u64 + 1,
                        WorldPoint::new(column as f32 * 18.0, row as f32 * 15.0),
                    )
                    .with_size(WorldSize::new(11.0, 7.0))
                    .with_type(example.node_types[index % example.node_types.len()]);
                    node.metadata.insert(
                        "caption".into(),
                        if index == 0 {
                            format!("{}\n{}", example.title, example.summary)
                        } else {
                            format!(
                                "{} node {}",
                                example.node_types[index % example.node_types.len()],
                                index + 1
                            )
                        },
                    );
                    nodes.push(node);
                }
                let mut edges = Vec::with_capacity(count.saturating_sub(1));
                for index in 0..count - 1 {
                    let mut edge =
                        Edge::new(index as u64 + 1, index as u64 + 2).with_id(index as u64 + 1);
                    edge.edge_type = example.edge_types[index % example.edge_types.len()].into();
                    edge.label = Some(example.edge_types[index % example.edge_types.len()].into());
                    if index + 2 == count {
                        edge.marker_end = Some(EdgeMarker::ArrowClosed);
                    }
                    edges.push(edge);
                }
                let mut renderer = GraphRenderer::default();
                for &kind in example.node_types.iter().filter(|kind| **kind != "default") {
                    renderer.register_node_type(
                        kind,
                        if kind == "diamond" {
                            diamond_node
                        } else {
                            square_node
                        },
                    );
                }
                Graph::builder()
                    .data(GraphData::new(nodes, edges))
                    .renderer(renderer)
                    .fit_on_load()
                    .show_handles(example.show_handles)
                    .build(cx)
                    .unwrap()
            });
            let on_event = example.on_event;
            cx.new(|cx| ExampleView::new(graph, on_event, cx))
        })
        .unwrap();
    });
}

#[macro_export]
macro_rules! catalog_example {
    ($example:expr) => {
        fn main() {
            support::run_catalog_example($example);
        }
    };
}

const SAMPLE_CAPACITY: usize = 120;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(250);

struct FrameMeter {
    previous_frame: Option<Instant>,
    last_publish: Instant,
    samples_ms: VecDeque<f32>,
    fps: f32,
    average_ms: f32,
    p95_ms: f32,
}

impl FrameMeter {
    fn new() -> Self {
        Self {
            previous_frame: None,
            last_publish: Instant::now(),
            samples_ms: VecDeque::with_capacity(SAMPLE_CAPACITY),
            fps: 0.0,
            average_ms: 0.0,
            p95_ms: 0.0,
        }
    }

    fn record_frame(&mut self, now: Instant) -> bool {
        if let Some(previous) = self.previous_frame.replace(now) {
            if self.samples_ms.len() == SAMPLE_CAPACITY {
                self.samples_ms.pop_front();
            }
            self.samples_ms
                .push_back(now.duration_since(previous).as_secs_f32() * 1_000.0);
        }

        if now.duration_since(self.last_publish) < PUBLISH_INTERVAL || self.samples_ms.is_empty() {
            return false;
        }
        self.last_publish = now;
        self.average_ms = self.samples_ms.iter().sum::<f32>() / self.samples_ms.len() as f32;
        self.fps = 1_000.0 / self.average_ms.max(0.001);
        let mut sorted: Vec<_> = self.samples_ms.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let p95_index = ((sorted.len() - 1) as f32 * 0.95).round() as usize;
        self.p95_ms = sorted[p95_index];
        true
    }
}

impl Render for FrameMeter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let meter = cx.entity();
        let ticker = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _, window, cx| {
                window.request_animation_frame();
                cx.update_entity(&meter, |meter, cx| {
                    if meter.record_frame(Instant::now()) {
                        cx.notify();
                    }
                });
            },
        )
        .absolute()
        .size(px(1.0));

        let label = if self.samples_ms.is_empty() {
            "FPS: measuring…".to_string()
        } else {
            format!(
                "FPS: {:.1}\navg: {:.2} ms\np95: {:.2} ms",
                self.fps, self.average_ms, self.p95_ms
            )
        };

        div()
            .absolute()
            .right(px(8.0))
            .bottom(px(8.0))
            .child(ticker)
            .child(
                div()
                    .bg(rgb(0xf7f7f7))
                    .border(px(1.0))
                    .border_color(rgb(0xcccccc))
                    .rounded(px(6.0))
                    .p(px(8.0))
                    .cursor_default()
                    .child(label),
            )
    }
}

pub struct ExampleView {
    graph: Entity<Graph>,
    frame_meter: Entity<FrameMeter>,
    event_log: Entity<EventLog>,
}

struct EventLog {
    graph: Entity<Graph>,
    on_event: Option<ExampleEventHandler>,
    last: String,
}

impl Render for EventLog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let graph = self.graph.clone();
        let on_event = self.on_event;
        let log = cx.entity();
        let poller = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                let events = cx.update_entity(&graph, |graph, cx| {
                    let events = graph.take_events();
                    if let Some(handler) = on_event {
                        for event in &events {
                            handler(graph, event, cx);
                        }
                    }
                    events
                });
                if let Some(event) = events.last() {
                    cx.update_entity(&log, |log, cx| {
                        log.last = format!("Last event: {event:?}");
                        cx.notify();
                    });
                }
            },
        )
        .absolute()
        .size(px(1.0));
        div()
            .absolute()
            .left(px(8.0))
            .bottom(px(8.0))
            .child(poller)
            .child(
                div()
                    .max_w(px(520.0))
                    .bg(rgb(0xf7f7f7))
                    .border(px(1.0))
                    .border_color(rgb(0xcccccc))
                    .rounded(px(6.0))
                    .p(px(8.0))
                    .child(self.last.clone()),
            )
    }
}

impl ExampleView {
    pub fn new(graph: Entity<Graph>, on_event: Option<ExampleEventHandler>, cx: &mut App) -> Self {
        Self {
            event_log: cx.new(|_| EventLog {
                graph: graph.clone(),
                on_event,
                last: "Interact with nodes, handles, edges, and the viewport".into(),
            }),
            graph,
            frame_meter: cx.new(|_| FrameMeter::new()),
        }
    }
}

impl Render for ExampleView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(self.event_log.clone())
            .child(self.frame_meter.clone())
    }
}

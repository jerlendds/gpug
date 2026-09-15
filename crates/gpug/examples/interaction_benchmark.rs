//! Native window for OS pointer profiling of the large_graph SmallWorld workload.
//! GPUG_INPUT_CASE=idle|empty|hover|pan|drag|pick, GPUG_NODE_COUNT=10000.
//! tools/benchmark_interaction.py supplies real X11 motion events.
use gpug::*;
use gpui::{prelude::*, *};
use std::time::Instant;

struct Replay {
    graph: Entity<Graph>,
    case: String,
    follow: bool,
    frame: usize,
    previous: Option<Instant>,
    intervals: Vec<f64>,
}
const WARMUP: usize = 60;
const SAMPLES: usize = 100000;
fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).round() as usize]
}
impl Render for Replay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.graph.update(cx, |graph, _| {
            for event in graph.take_events() {
                if matches!(&event, GraphEvent::SelectionChanged { .. }) {
                    println!("SELECTION_CHANGE");
                }
                if let GraphEvent::NodesChanged(changes) = event {
                    for change in changes {
                        if let NodeChange::Position {
                            id,
                            dragging: Some(dragging),
                            ..
                        } = change
                        {
                            println!("DRAG_CHANGE,{},{}", id.0, dragging);
                        }
                    }
                }
            }
            if self.follow || self.frame == 0 {
                let bounds = graph.content_bounds().unwrap();
                let zoom =
                    (400. / bounds.size.width.max(1.)).min(400. / bounds.size.height.max(1.));
                graph.set_center(
                    WorldPoint::new(
                        bounds.origin.x + bounds.size.width * 0.5,
                        bounds.origin.y + bounds.size.height * 0.5,
                    ),
                    window.viewport_size(),
                    zoom,
                );
            }
        });
        let entity = cx.entity();
        div().size_full().child(self.graph.clone()).child(
            canvas(
                |_, _, _| (),
                move |_, _, window, cx| {
                    cx.update_entity(&entity, |replay, cx| {
                        let now = Instant::now();
                        if let Some(previous) = replay.previous.replace(now) {
                            println!(
                                "FRAME,{:.3},{}",
                                now.duration_since(previous).as_secs_f64() * 1000.,
                                replay.graph.read(cx).layout_frame()
                            );
                            if replay.frame > WARMUP {
                                replay
                                    .intervals
                                    .push(now.duration_since(previous).as_secs_f64() * 1000.);
                            }
                        }
                        replay.frame += 1;
                        if replay.intervals.len() == SAMPLES {
                            let fps = SAMPLES as f64 * 1000. / replay.intervals.iter().sum::<f64>();
                            let p95 = percentile(&mut replay.intervals, 0.95);
                            let graph = replay.graph.read(cx);
                            println!(
                                "RESULT,{},{},{fps:.3},{p95:.3},{}",
                                replay.case,
                                graph.nodes().len(),
                                graph.layout_frame()
                            );
                            cx.quit();
                            return;
                        }
                        if replay.frame % 10 == 0 {
                            let graph = replay.graph.read(cx);
                            let viewport = graph.viewport();
                            println!(
                                "CAMERA,{},{},{}",
                                viewport.zoom(),
                                viewport.pan().x / px(1.),
                                viewport.pan().y / px(1.)
                            );
                            let p = graph.world_to_screen(graph.nodes()[0].position);
                            println!("POINTER_TARGET,{:.1},{:.1}", p.x / px(1.), p.y / px(1.));
                        }
                        window.request_animation_frame();
                    });
                },
            )
            .absolute()
            .size(px(1.)),
        )
    }
}
fn main() {
    let count = std::env::var("GPUG_NODE_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let case = std::env::var("GPUG_INPUT_CASE").unwrap_or_else(|_| "empty".into());
    assert!(
        ["idle", "empty", "hover", "pan", "drag", "pick", "zoom-in", "zoom-out"]
            .contains(&case.as_str())
    );
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                app_id: Some("gpug-interaction-benchmark".into()),
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1280.), px(800.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                let graph = cx.new(|cx| {
                    let mut data = SmallWorld::new(count)
                        .local_neighbors(3)
                        .shortcut_probability(0.00001)
                        .seed(42)
                        .generate();
                    if case == "drag" {
                        // Force this replay to measure node movement, not a nearby port.
                        for node in &mut data.nodes {
                            node.connectable = false;
                        }
                        for edge in &mut data.edges {
                            edge.reconnectable = false;
                        }
                    }
                    let all_edges = std::env::var("GPUG_ALL_EDGES").is_ok_and(|v| v == "1");
                    let style = if all_edges {
                        GraphStyle {
                            interactive_edge_budget: usize::MAX,
                            edge_lod_min_pixels: 0.,
                            frame_budget_ms: None,
                            ..GraphStyle::default()
                        }
                    } else {
                        GraphStyle::default()
                    };
                    let mut graph = Graph::builder()
                        .style(style)
                        .data(data)
                        .interactive_layout(true)
                        .build(cx)
                        .unwrap();
                    // Keep the graph at the center with a wide, unoccupied canvas margin.
                    let bounds = graph.content_bounds().unwrap();
                    let zoom =
                        (400. / bounds.size.width.max(1.)).min(400. / bounds.size.height.max(1.));
                    graph.set_center(
                        WorldPoint::new(
                            bounds.origin.x + bounds.size.width * 0.5,
                            bounds.origin.y + bounds.size.height * 0.5,
                        ),
                        size(px(1280.), px(800.)),
                        zoom,
                    );
                    if std::env::var("GPUG_STATIC").is_ok_and(|v| v == "1") {
                        graph.stop_layout();
                    }
                    graph
                });
                cx.new(|_| Replay {
                    graph,
                    case,
                    follow: std::env::var("GPUG_CAMERA").map_or(true, |v| v == "follow"),
                    frame: 0,
                    previous: None,
                    intervals: Vec::new(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

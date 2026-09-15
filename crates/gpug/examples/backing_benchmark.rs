//! Deterministic large-graph frame replay for the Vortex/Vec backing comparison.
//! Build with --release --features benchmark-backends. Select GPUG_BENCH_BACKEND
//! (vortex or vec), GPUG_BENCH_NODES (10000), and GPUG_BENCH_WORKLOAD (grid or hub).
//! Reports CPU paint-completion cadence, not GPU presentation timestamps.
use gpug::{analysis as data, Graph, WorldPoint};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, App, AppContext, Application, Bounds, Context,
    Entity, Render, Window, WindowBounds, WindowOptions,
};
use std::time::Instant;
#[path = "backing_benchmark/meter.rs"]
mod meter;

const WARMUP: usize = 60;
const SAMPLES: usize = 300;

struct Replay {
    graph: Entity<Graph>,
    frame: usize,
    previous: Option<Instant>,
    intervals_ms: Vec<f64>,
    count: usize,
    workload: String,
    layout: bool,
    sample_count: usize,
    meter: meter::FrameMeter,
}

impl Render for Replay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Frame-indexed poses give both backends exactly the same camera work,
        // even when one takes longer to produce a frame.
        if self.frame == 0 {
            println!(
                "VIEWPORT,{:.0},{:.0},{}",
                window.viewport_size().width / px(1.),
                window.viewport_size().height / px(1.),
                window.scale_factor()
            );
        }
        let phase = (self.frame % self.sample_count) as f32 / self.sample_count as f32
            * std::f32::consts::TAU;
        let side = (self.count as f32).sqrt().ceil() * 16.;
        let zoom = (window.viewport_size().height / px(1.)) / side * (1.4 + phase.sin() * 0.8);
        let center = WorldPoint::new(side * (0.5 + phase.cos() * 0.12), side * 0.5);
        self.graph.update(cx, |graph, cx| {
            if self.layout {
                let bounds = graph.content_bounds().expect("nonempty benchmark graph");
                let fitted = (window.viewport_size().width / px(1.) / bounds.size.width.max(1.))
                    .min(window.viewport_size().height / px(1.) / bounds.size.height.max(1.));
                graph.set_center(
                    WorldPoint::new(
                        bounds.origin.x + bounds.size.width * 0.5,
                        bounds.origin.y + bounds.size.height * 0.5,
                    ),
                    window.viewport_size(),
                    fitted * (1.4 + phase.sin() * 0.8),
                );
            } else {
                graph.set_center(center, window.viewport_size(), zoom);
            }
            cx.notify();
        });
        let entity = cx.entity();
        let fps = self.meter.fps().map_or_else(
            || "CPU FPS: measuring…".to_string(),
            |fps| format!("CPU FPS: {fps:.1}"),
        );
        let progress = format!(
            "Rendered frames: {} / {}",
            self.frame,
            WARMUP + self.sample_count
        );
        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        cx.update_entity(&entity, |replay, cx| {
                            let now = Instant::now();
                            if let Some(previous) = replay.previous.replace(now) {
                                replay.meter.record(now.duration_since(previous));
                                if replay.frame >= WARMUP {
                                    replay
                                        .intervals_ms
                                        .push(now.duration_since(previous).as_secs_f64() * 1000.);
                                }
                            }
                            replay.frame += 1;
                            if replay.intervals_ms.len() == replay.sample_count {
                                let sum: f64 = replay.intervals_ms.iter().sum();
                                replay.intervals_ms.sort_by(f64::total_cmp);
                                let p = |q: f64| {
                                    replay.intervals_ms
                                        [((replay.sample_count - 1) as f64 * q).round() as usize]
                                };
                                println!(
                                    "RESULT,{},{},{},{},{:.3},{:.3},{:.3},{:.3},{}",
                                    if data::column::uses_vec_backend() {
                                        "vec"
                                    } else {
                                        "vortex"
                                    },
                                    replay.workload,
                                    replay.count,
                                    replay.sample_count,
                                    replay.sample_count as f64 * 1000. / sum,
                                    p(0.5),
                                    p(0.95),
                                    p(0.99),
                                    replay.layout
                                );
                                let graph = replay.graph.read(cx);
                                let checksum = graph.nodes().iter().fold(0u64, |acc, node| {
                                    acc.wrapping_add(
                                        u64::from(node.position.x.to_bits()).wrapping_mul(31)
                                            ^ u64::from(node.position.y.to_bits()),
                                    )
                                });
                                println!(
                                    "FINAL,{},{},{}",
                                    graph.nodes().len(),
                                    graph.edges().len(),
                                    checksum
                                );
                                cx.quit();
                            } else {
                                window.request_animation_frame();
                                cx.notify();
                            }
                        });
                    },
                )
                .absolute()
                .size(px(1.)),
            )
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .p(px(10.))
                    .bg(rgb(0xf7f7f7))
                    .border(px(1.))
                    .border_color(rgb(0xcccccc))
                    .rounded(px(6.))
                    .flex()
                    .flex_col()
                    .child(fps)
                    .child(progress)
                    .child(if data::column::uses_vec_backend() {
                        "Vec backing"
                    } else {
                        "Vortex backing"
                    }),
            )
    }
}

// Fixed work per frame for the adjacency experiment, not a timing-dependent
// number of layout steps. The production layout's default budget is unchanged.
struct OneStep(gpug::ForceAtlas2);
impl gpug::Layout for OneStep {
    fn step(
        &mut self,
        positions: &mut [WorldPoint],
        edges: &[gpug::LayoutEdge],
    ) -> gpug::LayoutStatus {
        self.0.step(positions, edges)
    }
    fn use_frame_budget(&self) -> bool {
        false
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::var("GPUG_BENCH_NODES")
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(10_000usize);
    if count < 4 {
        return Err("GPUG_BENCH_NODES must be at least 4".into());
    }
    let workload = std::env::var("GPUG_BENCH_WORKLOAD").unwrap_or_else(|_| "grid".into());
    if workload != "grid" && workload != "hub" {
        return Err("workload must be grid or hub".into());
    }
    let sample_count = std::env::var("GPUG_BENCH_SAMPLES")
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(SAMPLES);
    if sample_count < 60 {
        return Err("GPUG_BENCH_SAMPLES must be at least 60".into());
    }
    let layout = std::env::var("GPUG_BENCH_LAYOUT").is_ok_and(|v| v == "1");
    let start = Instant::now();
    let columns = (count as f64).sqrt().ceil() as usize;
    let nodes = (0..count)
        .map(|i| {
            data::Node::new(
                i as u64,
                data::WorldRect {
                    x: (i % columns) as f64 * 16.,
                    y: (i / columns) as f64 * 16.,
                    width: 6.,
                    height: 6.,
                },
            )
        })
        .collect();
    let mut edges = Vec::with_capacity(count * 3);
    for i in 0..count {
        for distance in [1, 2, 3] {
            let target = if workload == "hub" {
                (i + distance) % 4
            } else {
                (i + distance) % count
            };
            edges.push(data::Edge::new(
                (i * 3 + distance - 1) as u64,
                i as u64,
                target as u64,
            ));
        }
    }
    let snapshot = data::GraphSnapshot::new(nodes, edges)?;
    println!("SOURCE,{:.3}", start.elapsed().as_secs_f64() * 1000.);
    Application::new().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("gpug-backing-benchmark".into()),
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1280.), px(800.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                let start = Instant::now();
                let graph = cx.new(|cx| {
                    let mut graph = Graph::builder()
                        .snapshot(&snapshot)
                        .unwrap()
                        .interactive_layout(false)
                        .layout(OneStep(gpug::ForceAtlas2::default()))
                        .build(cx)
                        .unwrap();
                    if layout {
                        graph.start_layout();
                    }
                    graph
                });
                println!("INSTALL,{:.3}", start.elapsed().as_secs_f64() * 1000.);
                cx.new(|_| Replay {
                    graph,
                    frame: 0,
                    previous: None,
                    intervals_ms: Vec::with_capacity(sample_count),
                    count,
                    workload,
                    layout,
                    sample_count,
                    meter: meter::FrameMeter::default(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
    Ok(())
}

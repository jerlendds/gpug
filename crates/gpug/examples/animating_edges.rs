//! A paint-time edge animation gallery.
use gpui::{
    App, AppContext, Application, Bounds, Context, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Path, Pixels, Point, Render,
    Styled, Window, WindowOptions, canvas, div, fill, point, px, rgb, rgba, size,
};
use std::time::Instant;

const NODES: [(f32, f32); 8] = [
    (0.10, 0.18),
    (0.38, 0.12),
    (0.68, 0.20),
    (0.88, 0.43),
    (0.72, 0.76),
    (0.42, 0.84),
    (0.14, 0.72),
    (0.48, 0.48),
];
const EDGES: [(usize, usize); 10] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 4),
    (4, 5),
    (5, 6),
    (6, 0),
    (0, 7),
    (2, 7),
    (5, 7),
];

struct AnimationGallery {
    started: Instant,
    nodes: [(f32, f32); 8],
    dragging: Option<usize>,
}

impl AnimationGallery {
    const INSET: f32 = 70.0;

    fn screen_positions(&self, window: &Window) -> [Point<Pixels>; 8] {
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - Self::INSET * 2.0).max(1.0);
        let height = (f32::from(viewport.height) - Self::INSET * 2.0).max(1.0);
        self.nodes
            .map(|(x, y)| point(px(Self::INSET + x * width), px(Self::INSET + y * height)))
    }

    fn node_at(&self, position: Point<Pixels>, window: &Window) -> Option<usize> {
        self.screen_positions(window).iter().position(|center| {
            let dx = f32::from(position.x - center.x);
            let dy = f32::from(position.y - center.y);
            dx * dx + dy * dy <= 20.0 * 20.0
        })
    }

    fn move_dragged_node(&mut self, position: Point<Pixels>, window: &Window) {
        let Some(index) = self.dragging else { return };
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - Self::INSET * 2.0).max(1.0);
        let height = (f32::from(viewport.height) - Self::INSET * 2.0).max(1.0);
        self.nodes[index] = (
            ((f32::from(position.x) - Self::INSET) / width).clamp(0.0, 1.0),
            ((f32::from(position.y) - Self::INSET) / height).clamp(0.0, 1.0),
        );
    }
}

fn mix(a: Point<Pixels>, b: Point<Pixels>, t: f32) -> Point<Pixels> {
    point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn edge_path(a: Point<Pixels>, b: Point<Pixels>, width: f32) -> Path<Pixels> {
    let direction = point(b.x - a.x, b.y - a.y);
    let length = direction.magnitude() as f32;
    let mut path = Path::new(a);
    if length < 0.001 {
        return path;
    }
    let normal = point(-direction.y, direction.x) * (width * 0.5 / length);
    let uv = (point(0., 1.), point(0., 1.), point(0., 1.));
    path.push_triangle(
        (
            point(a.x + normal.x, a.y + normal.y),
            point(a.x - normal.x, a.y - normal.y),
            point(b.x + normal.x, b.y + normal.y),
        ),
        uv,
    );
    path.push_triangle(
        (
            point(b.x + normal.x, b.y + normal.y),
            point(a.x - normal.x, a.y - normal.y),
            point(b.x - normal.x, b.y - normal.y),
        ),
        uv,
    );
    path
}

fn paint_edge(window: &mut Window, a: Point<Pixels>, b: Point<Pixels>, width: f32, color: u32) {
    window.paint_path(edge_path(a, b, width), rgba(color));
}

impl Render for AnimationGallery {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let started = self.started;
        let nodes = self.nodes;
        let graph = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                window.request_animation_frame();
                let time = started.elapsed().as_secs_f32();
                let inset = AnimationGallery::INSET;
                let w = (f32::from(bounds.size.width) - inset * 2.0).max(1.0);
                let h = (f32::from(bounds.size.height) - inset * 2.0).max(1.0);
                let positions = nodes.map(|(x, y)| {
                    point(
                        bounds.left() + px(inset + x * w),
                        bounds.top() + px(inset + y * h),
                    )
                });
                for (edge_index, &(s, t)) in EDGES.iter().enumerate() {
                    // Physical deformation effects paint their complete edge below;
                    // a static foundation would look like a second connection.
                    if matches!(edge_index, 1 | 4 | 5) {
                        continue;
                    }
                    paint_edge(window, positions[s], positions[t], 2.0, 0x33415555);
                }

                // Custom GPUI-painted element traveling source -> target.
                let (a, b) = (positions[0], positions[1]);
                paint_edge(window, a, b, 2.0, 0x64748bff);
                let courier = mix(a, b, (time * 0.32).fract());
                window.paint_quad(fill(
                    Bounds::centered_at(courier, size(px(14.0), px(14.0))),
                    rgb(0xec4899),
                ));

                // Stroke dash offset: phase advancement prevents a node-anchored dash.
                let (a, b) = (positions[1], positions[2]);
                let phase = (time * 0.16).fract();
                for dash in 0..9 {
                    let start = ((dash as f32 / 9.0) + phase).fract();
                    let end = (start + 0.055).min(1.0);
                    paint_edge(window, mix(a, b, start), mix(a, b, end), 3.0, 0x8b5cf6ff);
                }

                // Moving gradient, represented by short independently colored stroke spans.
                let (a, b) = (positions[2], positions[3]);
                let head = (time * 0.28).fract();
                for segment in 0..32 {
                    let t0 = segment as f32 / 32.0;
                    let t1 = (segment + 1) as f32 / 32.0;
                    let distance = ((t0 - head + 1.5).fract() - 0.5).abs();
                    let glow = (1.0 - distance * 7.0).clamp(0.0, 1.0);
                    paint_edge(
                        window,
                        mix(a, b, t0),
                        mix(a, b, t1),
                        3.0 + glow * 2.0,
                        0x06b6d400 | (70.0 + glow * 185.0) as u32,
                    );
                }

                // Breathing width and opacity.
                let breath = (time * 2.2).sin() * 0.5 + 0.5;
                paint_edge(
                    window,
                    positions[3],
                    positions[4],
                    2.0 + breath * 5.0,
                    0x22c55e00 | (80.0 + breath * 175.0) as u32,
                );

                // Simulated topology pulse every four seconds; spring energy decays e^-kt.
                let age = time % 4.0;
                let displacement = (age * 28.0).sin() * (-3.2 * age).exp() * 24.0;
                let (a, b) = (positions[4], positions[5]);
                let midpoint = mix(a, b, 0.5);
                let kicked = point(midpoint.x, midpoint.y + px(displacement));
                paint_edge(window, a, kicked, 3.0, 0x64748bff);
                paint_edge(window, kicked, b, 3.0, 0x64748bff);

                // Elastic creation overshoots; deletion retracts toward the source.
                let cycle = time % 5.0;
                let progress = if cycle < 2.2 {
                    let x = cycle / 2.2;
                    1.0 - (-5.0 * x).exp() * (x * 11.0).cos()
                } else if cycle < 4.0 {
                    1.0
                } else {
                    (1.0 - (cycle - 4.0)).max(0.0)
                };
                let (a, b) = (positions[5], positions[6]);
                paint_edge(
                    window,
                    a,
                    mix(a, b, progress.clamp(0.0, 1.08)),
                    3.0,
                    0x64748bff,
                );

                // Operation charge fills source -> target on two edges.
                for (source, target, offset) in [(6, 0, 0.0), (5, 7, 0.5)] {
                    let charge = (time * 0.18 + offset) % 1.0;
                    let (a, b) = (positions[source], positions[target]);
                    paint_edge(window, a, mix(a, b, charge), 5.0, 0x0ea5e9ff);
                }

                // Combined courier/breathing edge and reverse gradient edge.
                let (a, b) = (positions[0], positions[7]);
                paint_edge(window, a, b, 2.0 + breath * 2.0, 0xa855f7bb);
                let pulse = mix(a, b, (time * 0.45).fract());
                window.paint_quad(fill(
                    Bounds::centered_at(pulse, size(px(10.0), px(10.0))),
                    rgb(0xf0abfc),
                ));
                let (a, b) = (positions[2], positions[7]);
                for segment in 0..24 {
                    let t0 = segment as f32 / 24.0;
                    let t1 = (segment + 1) as f32 / 24.0;
                    let wave =
                        ((t0 + time * 0.35).fract() * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                    paint_edge(
                        window,
                        mix(a, b, t0),
                        mix(a, b, t1),
                        2.0 + wave * 3.0,
                        0xfacc1500 | (70.0 + wave * 185.0) as u32,
                    );
                }

                for center in positions {
                    window.paint_quad(fill(
                        Bounds::centered_at(center, size(px(28.0), px(28.0))),
                        rgb(0x0f172a),
                    ));
                    window.paint_quad(fill(
                        Bounds::centered_at(center, size(px(18.0), px(18.0))),
                        rgb(0xf8fafc),
                    ));
                }
            },
        )
        .size_full();

        div()
            .relative()
            .size_full()
            .bg(rgb(0xf8fafc))
            .child(graph)
            .on_mouse_down(
                MouseButton::Left,
                _cx.listener(|gallery, event: &MouseDownEvent, window, cx| {
                    gallery.dragging = gallery.node_at(event.position, window);
                    if gallery.dragging.is_some() {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(_cx.listener(|gallery, event: &MouseMoveEvent, window, cx| {
                if event.dragging() && gallery.dragging.is_some() {
                    gallery.move_dragged_node(event.position, window);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                _cx.listener(|gallery, _: &MouseUpEvent, _, cx| {
                    if gallery.dragging.take().is_some() {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .child(div().absolute().left(px(16.0)).top(px(14.0)).text_color(rgb(0x334155)).child("Animating edges — drag any node · courier · dash offset · gradient · breathing · vibration · elastic snap · charging"))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Animating Edges".into()),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| AnimationGallery {
                    started: Instant::now(),
                    nodes: NODES,
                    dragging: None,
                })
            },
        )
        .unwrap();
    });
}

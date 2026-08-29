use std::borrow::Cow;

use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, point, px, radians, rgb, size, svg, App, AppContext, Application, AssetSource,
    Bounds, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent,
    SharedString, Styled, Transformation, Window, WindowOptions,
};

const NODE_SIZE: f32 = 240.0;
const HANDLE_DISTANCE: f32 = 95.0;
const HANDLE_HIT_SIZE: f32 = 30.0;

const BLUE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><line x1="120" y1="65" x2="120" y2="25" stroke="#fff" stroke-width="2"/><circle cx="120" cy="25" r="10" fill="#fff"/></svg>"##;
const SHADOW_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><rect x="56" y="64" width="128" height="119" rx="19" fill="#fff"/></svg>"##;
const FILL_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><rect x="60" y="65" width="120" height="110" rx="16" fill="#fff"/><circle cx="60" cy="120" r="7" fill="#fff"/><circle cx="180" cy="120" r="7" fill="#fff"/></svg>"##;
const OUTLINE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><rect x="60" y="65" width="120" height="110" rx="16" fill="none" stroke="#fff" stroke-width="2"/><circle cx="60" cy="120" r="7" fill="none" stroke="#fff" stroke-width="2"/><circle cx="180" cy="120" r="7" fill="none" stroke="#fff" stroke-width="2"/></svg>"##;
const TEXT_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><text x="120" y="112" text-anchor="middle" font-family="sans-serif" font-size="24" fill="#fff">Rotate</text><text x="120" y="146" text-anchor="middle" font-family="sans-serif" font-size="24" fill="#fff">Me!</text></svg>"##;
const LAYERS: [(&str, u32); 5] = [
    ("node-shadow.svg", 0x00000018),
    ("node-blue.svg", 0x326bddff),
    ("node-fill.svg", 0xffffffff),
    ("node-outline.svg", 0xa8a8a8ff),
    ("node-text.svg", 0x111111ff),
];

struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "node-blue.svg" => Some(Cow::Borrowed(BLUE_SVG)),
            "node-shadow.svg" => Some(Cow::Borrowed(SHADOW_SVG)),
            "node-fill.svg" => Some(Cow::Borrowed(FILL_SVG)),
            "node-outline.svg" => Some(Cow::Borrowed(OUTLINE_SVG)),
            "node-text.svg" => Some(Cow::Borrowed(TEXT_SVG)),
            _ => None,
        })
    }
    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Copy)]
struct NodeState {
    position: Point<f32>,
    rotation: f32,
}

#[derive(Clone, Copy)]
enum Gesture {
    Rotate {
        node: usize,
        previous_angle: f32,
    },
    Move {
        node: usize,
        pointer_start: Point<Pixels>,
        position_start: Point<f32>,
    },
    Pan {
        pointer_start: Point<Pixels>,
        pan_start: Point<Pixels>,
    },
}

struct RotatableNodeExample {
    nodes: [NodeState; 2],
    zoom: f32,
    pan: Point<Pixels>,
    gesture: Option<Gesture>,
}

impl RotatableNodeExample {
    fn viewport_center(&self, window: &Window) -> Point<Pixels> {
        let viewport = window.viewport_size();
        point(
            viewport.width * 0.5 + self.pan.x,
            viewport.height * 0.5 + self.pan.y,
        )
    }

    fn node_center(&self, node: usize, window: &Window) -> Point<Pixels> {
        let center = self.viewport_center(window);
        point(
            center.x + px(self.nodes[node].position.x * self.zoom),
            center.y + px(self.nodes[node].position.y * self.zoom),
        )
    }

    fn pointer_angle(position: Point<Pixels>, center: Point<Pixels>) -> f32 {
        f32::from(position.y - center.y).atan2(f32::from(position.x - center.x))
    }

    fn unwrap_delta(delta: f32) -> f32 {
        (delta + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    }

    fn handle_center(&self, node: usize, center: Point<Pixels>) -> Point<Pixels> {
        let rotation = self.nodes[node].rotation;
        point(
            center.x + px(rotation.sin() * HANDLE_DISTANCE * self.zoom),
            center.y - px(rotation.cos() * HANDLE_DISTANCE * self.zoom),
        )
    }

    fn node_contains(&self, node: usize, pointer: Point<Pixels>, center: Point<Pixels>) -> bool {
        let dx = f32::from(pointer.x - center.x);
        let dy = f32::from(pointer.y - center.y);
        let (sin, cos) = self.nodes[node].rotation.sin_cos();
        let local = point(cos * dx + sin * dy, -sin * dx + cos * dy);
        local.x.abs() <= 60.0 * self.zoom && local.y.abs() <= 55.0 * self.zoom
    }
}

impl Render for RotatableNodeExample {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let centers = [self.node_center(0, window), self.node_center(1, window)];
        let handles = [
            self.handle_center(0, centers[0]),
            self.handle_center(1, centers[1]),
        ];
        let nodes = self.nodes;
        let zoom = self.zoom;
        let gesture = self.gesture;

        let background = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let spacing = 40.0 * zoom;
                let columns = (f32::from(bounds.size.width) / spacing).ceil() as usize;
                let rows = (f32::from(bounds.size.height) / spacing).ceil() as usize;
                for row in 0..=rows {
                    for column in 0..=columns {
                        window.paint_quad(gpui::fill(
                            Bounds::centered_at(
                                point(
                                    bounds.left() + px(column as f32 * spacing + 12.0),
                                    bounds.top() + px(row as f32 * spacing + 30.0),
                                ),
                                size(px(2.0), px(2.0)),
                            ),
                            rgb(0xaab0ba),
                        ));
                    }
                }

                let source = point(
                    centers[0].x + px(nodes[0].rotation.cos() * 60.0 * zoom),
                    centers[0].y + px(nodes[0].rotation.sin() * 60.0 * zoom),
                );
                let target = point(
                    centers[1].x - px(nodes[1].rotation.cos() * 60.0 * zoom),
                    centers[1].y - px(nodes[1].rotation.sin() * 60.0 * zoom),
                );
                let elbow = (source.x + target.x) * 0.5;
                for segment in [
                    Bounds::from_corners(
                        point(source.x.min(elbow), source.y - px(1.0)),
                        point(source.x.max(elbow), source.y + px(1.0)),
                    ),
                    Bounds::from_corners(
                        point(elbow - px(1.0), source.y.min(target.y)),
                        point(elbow + px(1.0), source.y.max(target.y)),
                    ),
                    Bounds::from_corners(
                        point(elbow.min(target.x), target.y - px(1.0)),
                        point(elbow.max(target.x), target.y + px(1.0)),
                    ),
                ] {
                    window.paint_quad(gpui::fill(segment, rgb(0xb0b3bb)));
                }
            },
        );

        div()
            .id("rotatable-node-example")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0xf8f9fb))
            .child(background)
            .children((0..2).flat_map(|node| {
                LAYERS.map(move |(path, color)| {
                    svg()
                        .path(path)
                        .absolute()
                        .left(centers[node].x - px(NODE_SIZE * 0.5))
                        .top(centers[node].y - px(NODE_SIZE * 0.5))
                        .size(px(NODE_SIZE))
                        .text_color(gpui::rgba(color))
                        .with_transformation(
                            Transformation::rotate(radians(nodes[node].rotation))
                                .with_scaling(size(zoom, zoom)),
                        )
                })
            }))
            .children((0..2).map(|node| {
                div()
                    .id(("rotation-handle", node))
                    .absolute()
                    .left(handles[node].x - px(HANDLE_HIT_SIZE * 0.5))
                    .top(handles[node].y - px(HANDLE_HIT_SIZE * 0.5))
                    .size(px(HANDLE_HIT_SIZE))
                    .rounded_full()
                    .cursor(CursorStyle::DragLink)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.gesture = Some(Gesture::Rotate {
                                node,
                                previous_angle: Self::pointer_angle(
                                    event.position,
                                    this.node_center(node, window),
                                ),
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if let Some(node) = (0..2).rev().find(|&node| {
                        this.node_contains(node, event.position, this.node_center(node, window))
                    }) {
                        this.gesture = Some(Gesture::Move {
                            node,
                            pointer_start: event.position,
                            position_start: this.nodes[node].position,
                        });
                    } else {
                        this.gesture = Some(Gesture::Pan {
                            pointer_start: event.position,
                            pan_start: this.pan,
                        });
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                match this.gesture {
                    Some(Gesture::Rotate {
                        node,
                        previous_angle,
                    }) => {
                        let angle =
                            Self::pointer_angle(event.position, this.node_center(node, window));
                        this.nodes[node].rotation += Self::unwrap_delta(angle - previous_angle);
                        this.gesture = Some(Gesture::Rotate {
                            node,
                            previous_angle: angle,
                        });
                    }
                    Some(Gesture::Move {
                        node,
                        pointer_start,
                        position_start,
                    }) => {
                        this.nodes[node].position = point(
                            position_start.x
                                + f32::from(event.position.x - pointer_start.x) / this.zoom,
                            position_start.y
                                + f32::from(event.position.y - pointer_start.y) / this.zoom,
                        );
                    }
                    Some(Gesture::Pan {
                        pointer_start,
                        pan_start,
                    }) => {
                        this.pan = point(
                            pan_start.x + event.position.x - pointer_start.x,
                            pan_start.y + event.position.y - pointer_start.y,
                        );
                    }
                    None => return,
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.gesture.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let dy = f32::from(event.delta.pixel_delta(px(16.0)).y);
                if dy == 0.0 {
                    return;
                }
                let factor = 1.04_f32.powf((dy / 16.0).abs().max(0.01));
                let factor = if dy > 0.0 { factor } else { factor.recip() };
                let old_zoom = this.zoom;
                let new_zoom = (old_zoom * factor).clamp(0.001, 256.0);
                let viewport = window.viewport_size();
                let viewport_center = point(viewport.width * 0.5, viewport.height * 0.5);
                let scale = new_zoom / old_zoom;
                this.pan = point(
                    event.position.x
                        - viewport_center.x
                        - (event.position.x - viewport_center.x - this.pan.x) * scale,
                    event.position.y
                        - viewport_center.y
                        - (event.position.y - viewport_center.y - this.pan.y) * scale,
                );
                this.zoom = new_zoom;
                cx.notify();
            }))
            .when(matches!(gesture, Some(Gesture::Rotate { .. })), |el| {
                el.cursor(CursorStyle::DragLink)
            })
            .when(
                matches!(gesture, Some(Gesture::Move { .. } | Gesture::Pan { .. })),
                |el| el.cursor(CursorStyle::ClosedHand),
            )
    }
}

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| RotatableNodeExample {
                nodes: [
                    NodeState {
                        position: point(-110.0, 0.0),
                        rotation: 0.0,
                    },
                    NodeState {
                        position: point(110.0, 0.0),
                        rotation: 0.0,
                    },
                ],
                zoom: 1.0,
                pan: point(px(0.0), px(0.0)),
                gesture: None,
            })
        })
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::RotatableNodeExample;
    #[test]
    fn angle_delta_unwraps_across_pi() {
        let delta = RotatableNodeExample::unwrap_delta(-std::f32::consts::TAU + 0.2);
        assert!((delta - 0.2).abs() < 0.0001);
    }
}

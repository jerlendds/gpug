use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpug::{
    Edge, Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeShape, WorldPoint, WorldSize,
};
use gpui::{
    App, AppContext, Application, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Path, Pixels, Point,
    Render, Styled, Window, WindowOptions, canvas, div, point, px, rgb, rgba,
};

#[derive(Clone)]
struct ColorValue {
    hex: String,
    rgb: u32,
}

impl Default for ColorValue {
    fn default() -> Self {
        Self {
            hex: "#c01c28".into(),
            rgb: 0xc01c28,
        }
    }
}

fn parse_hex(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('#').unwrap_or(value);
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

struct HexInput {
    value: String,
    cursor: usize,
    focus: FocusHandle,
    color: Arc<Mutex<ColorValue>>,
}

impl HexInput {
    fn new(color: Arc<Mutex<ColorValue>>, cx: &mut App) -> Self {
        let value = color.lock().expect("color lock poisoned").hex.clone();
        Self {
            cursor: value.len(),
            value,
            focus: cx.focus_handle().tab_stop(true),
            color,
        }
    }

    fn mouse_down(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        cx.stop_propagation();
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "left" => {
                self.cursor = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index)
            }
            "right" => {
                if let Some(next) = self.value[self.cursor..].chars().next() {
                    self.cursor += next.len_utf8();
                }
            }
            "home" => self.cursor = 0,
            "end" => self.cursor = self.value.len(),
            "backspace" => {
                if let Some(previous) = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(index, _)| index)
                {
                    self.value.replace_range(previous..self.cursor, "");
                    self.cursor = previous;
                }
            }
            "delete" => {
                if let Some(next) = self.value[self.cursor..].chars().next() {
                    self.value
                        .replace_range(self.cursor..self.cursor + next.len_utf8(), "")
                }
            }
            _ if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.alt =>
            {
                let Some(text) = &event.keystroke.key_char else {
                    return;
                };
                if text.chars().all(|ch| ch == '#' || ch.is_ascii_hexdigit())
                    && self.value.len() + text.len() <= 7
                {
                    self.value.insert_str(self.cursor, text);
                    self.cursor += text.len();
                }
            }
            _ => return,
        }
        if let Some(rgb) = parse_hex(&self.value) {
            *self.color.lock().expect("color lock poisoned") = ColorValue {
                hex: format!("#{rgb:06x}"),
                rgb,
            };
        }
        cx.stop_propagation();
        cx.refresh_windows();
        cx.notify();
    }
}

impl Focusable for HexInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for HexInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_key_down(cx.listener(Self::key_down))
            .w_full()
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .whitespace_nowrap()
            .overflow_hidden()
            .rounded(px(5.0))
            .border(px(1.0))
            .border_color(rgb(if self.focus.is_focused(window) {
                0xff0072
            } else {
                0xcbd5e1
            }))
            .bg(rgb(0xffffff))
            .cursor_text()
            .child(self.value.clone())
    }
}

fn card(title: &str, zoom: f32, body: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .p(px(0.7 * zoom))
        .gap(px(0.5 * zoom))
        .flex()
        .flex_col()
        .text_size(px(1.55 * zoom))
        .rounded(px(0.7 * zoom))
        .border(px(1.0))
        .border_color(rgb(0xcbd5e1))
        .bg(rgb(0xffffff))
        .shadow_sm()
        .overflow_hidden()
        .child(title.to_string())
        .child(div().flex_1().overflow_hidden().child(body))
}

fn mix(a: Point<Pixels>, b: Point<Pixels>, t: f32) -> Point<Pixels> {
    point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn node_boundary(
    center: Point<Pixels>,
    toward: Point<Pixels>,
    half_width: f32,
    half_height: f32,
) -> Point<Pixels> {
    let dx = (toward.x - center.x) / px(1.0);
    let dy = (toward.y - center.y) / px(1.0);
    let tx = if dx.abs() > 0.001 {
        half_width / dx.abs()
    } else {
        f32::INFINITY
    };
    let ty = if dy.abs() > 0.001 {
        half_height / dy.abs()
    } else {
        f32::INFINITY
    };
    let scale = tx.min(ty);
    point(center.x + px(dx * scale), center.y + px(dy * scale))
}

fn paint_segment(window: &mut Window, a: Point<Pixels>, b: Point<Pixels>) {
    let direction = point(b.x - a.x, b.y - a.y);
    let length = direction.magnitude() as f32;
    if length <= 0.001 {
        return;
    }
    let normal = point(-direction.y, direction.x) * (0.8 / length);
    let st = (point(0., 1.), point(0., 1.), point(0., 1.));
    let mut path = Path::new(a);
    path.push_triangle(
        (
            point(a.x + normal.x, a.y + normal.y),
            point(a.x - normal.x, a.y - normal.y),
            point(b.x + normal.x, b.y + normal.y),
        ),
        st,
    );
    path.push_triangle(
        (
            point(b.x + normal.x, b.y + normal.y),
            point(a.x - normal.x, a.y - normal.y),
            point(b.x - normal.x, b.y - normal.y),
        ),
        st,
    );
    window.paint_path(path, rgba(0x64748baa));
}

struct CustomNodes {
    graph: Entity<Graph>,
    input: Entity<HexInput>,
    color: Arc<Mutex<ColorValue>>,
    started: Instant,
}

impl Render for CustomNodes {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let graph = self.graph.clone();
        let started = self.started;
        let animated_edges = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                let phase = (started.elapsed().as_secs_f32() * 0.28).fract();
                cx.read_entity(&graph, |graph, _| {
                    for edge in graph.edges() {
                        let Some(source) = graph.node(edge.source) else {
                            continue;
                        };
                        let Some(target) = graph.node(edge.target) else {
                            continue;
                        };
                        let source_center = graph
                            .flow_to_screen_position(graph.editor().node_center_absolute(source));
                        let target_center = graph
                            .flow_to_screen_position(graph.editor().node_center_absolute(target));
                        let zoom = graph.viewport().zoom();
                        let a = node_boundary(
                            source_center,
                            target_center,
                            source.size.width * zoom * 0.5,
                            source.size.height * zoom * 0.5,
                        );
                        let b = node_boundary(
                            target_center,
                            source_center,
                            target.size.width * zoom * 0.5,
                            target.size.height * zoom * 0.5,
                        );
                        for dash in 0..12 {
                            let start = ((dash as f32 / 12.0) + phase).fract();
                            let end = (start + 0.045).min(1.0);
                            paint_segment(window, mix(a, b, start), mix(a, b, end));
                        }
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let color = self.color.lock().expect("color lock poisoned").clone();
        let panel = div()
            .absolute()
            .top(px(12.0))
            .right(px(12.0))
            .px_3()
            .py_2()
            .gap_2()
            .flex()
            .items_center()
            .rounded(px(8.0))
            .border(px(1.0))
            .border_color(rgb(0xcbd5e1))
            .bg(rgb(0xffffff))
            .shadow_sm()
            .child(div().size(px(18.0)).rounded_full().bg(rgb(color.rgb)))
            .child(format!("Selected color: {}", color.hex));

        let _ = &self.input;
        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(animated_edges)
            .child(panel)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Custom Nodes".into()),
                ..Default::default()
            },
            |_, cx| {
                let color = Arc::new(Mutex::new(ColorValue::default()));
                let input = cx.new(|cx| HexInput::new(color.clone(), cx));
                let nodes = vec![
                    Node::new(1_u64, WorldPoint::new(0.0, 5.0))
                        .with_size(WorldSize::new(18.0, 7.0))
                        .with_type("label"),
                    Node::new(2_u64, WorldPoint::new(28.0, 5.0))
                        .with_size(WorldSize::new(24.0, 12.0))
                        .with_type("color_picker"),
                    Node::new(3_u64, WorldPoint::new(62.0, 0.0))
                        .with_size(WorldSize::new(18.0, 7.0))
                        .with_type("output_a"),
                    Node::new(4_u64, WorldPoint::new(62.0, 13.0))
                        .with_size(WorldSize::new(18.0, 7.0))
                        .with_type("output_b"),
                ];
                let edges = vec![
                    Edge::new(1_u64, 2_u64).with_id(1_u64),
                    Edge::new(2_u64, 3_u64).with_id(2_u64),
                    Edge::new(2_u64, 4_u64).with_id(3_u64),
                ];

                let mut renderer = GraphRenderer::default();
                let mut style = renderer.style().clone();
                style.edge_color = 0xffffff;
                renderer.set_style(style);
                for kind in ["label", "color_picker", "output_a", "output_b"] {
                    renderer.register_node_type(kind, |node: &Node, zoom, _: &gpug::GraphStyle| {
                        NodeAppearance {
                            color: 0xffffff,
                            radius_pixels: node.size.width * zoom * 0.5,
                            shape: NodeShape::None,
                        }
                    });
                }
                renderer.register_node_content("label", move |_: &Node, zoom| {
                    card(
                        "An input node",
                        zoom,
                        div().flex().items_center().child("Feeds the picker"),
                    )
                    .into_any_element()
                });
                let picker_input = input.clone();
                let picker_color = color.clone();
                renderer.register_node_content("color_picker", move |_: &Node, zoom| {
                    let current = picker_color.lock().expect("color lock poisoned").clone();
                    card(
                        "Custom Color Picker Node",
                        zoom,
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(0.5 * zoom))
                            .child(
                                div()
                                    .h(px(2.4 * zoom))
                                    .rounded(px(0.8 * zoom))
                                    .bg(rgb(current.rgb)),
                            )
                            .child(div().flex_1().child(picker_input.clone())),
                    )
                    .into_any_element()
                });
                renderer.register_node_content("output_a", move |_: &Node, zoom| {
                    card("Output A", zoom, div()).into_any_element()
                });
                renderer.register_node_content("output_b", move |_: &Node, zoom| {
                    card("Output B", zoom, div()).into_any_element()
                });

                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, edges))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| CustomNodes {
                    graph,
                    input,
                    color,
                    started: Instant::now(),
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
    fn parses_complete_six_digit_hex_colors() {
        assert_eq!(parse_hex("#c01c28"), Some(0xc01c28));
        assert_eq!(parse_hex("1e90ff"), Some(0x1e90ff));
        assert_eq!(parse_hex("#fff"), None);
        assert_eq!(parse_hex("#xxxxxx"), None);
    }

    #[test]
    fn rejects_non_hex_input_characters() {
        assert!(
            "#a0B9fF"
                .chars()
                .all(|ch| ch == '#' || ch.is_ascii_hexdigit())
        );
        assert!(
            !"#red123"
                .chars()
                .all(|ch| ch == '#' || ch.is_ascii_hexdigit())
        );
    }
}

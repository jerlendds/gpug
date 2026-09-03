use gpug::{
    Edge, Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeShape, WorldPoint, WorldSize,
};
use gpui::{
    div, prelude::*, px, rgb, App, AppContext, Application, Context, Entity, FocusHandle,
    Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Render, Window, WindowOptions,
};

const EDITABLE_NODE: &str = "editable-node";

fn parse_color(value: &str) -> Option<u32> {
    let digits = value.trim().strip_prefix('#').unwrap_or(value.trim());
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

fn update_node(graph: &Entity<Graph>, cx: &mut App, update: impl FnOnce(&mut Node)) {
    cx.update_entity(graph, |graph, graph_cx| {
        let mut nodes = graph.nodes().to_vec();
        let edges = graph.edges().to_vec();
        if let Some(node) = nodes.iter_mut().find(|node| node.id.0 == 1) {
            update(node);
            graph
                .set_data(GraphData::new(nodes, edges))
                .expect("valid node update");
            graph_cx.notify();
        }
    });
}

#[derive(Clone, Copy)]
enum Field {
    Color,
    Label,
}

struct TextInput {
    graph: Entity<Graph>,
    field: Field,
    value: String,
    cursor: usize,
    focus: FocusHandle,
}

impl TextInput {
    fn new(graph: Entity<Graph>, field: Field, value: &str, cx: &mut App) -> Self {
        Self {
            graph,
            field,
            value: value.into(),
            cursor: value.len(),
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    fn publish(&self, cx: &mut App) {
        let value = self.value.clone();
        let field = self.field;
        update_node(&self.graph, cx, move |node| match field {
            Field::Color => {
                if parse_color(&value).is_some() {
                    node.metadata.insert("color".into(), value);
                }
            }
            Field::Label => {
                node.metadata.insert("caption".into(), value);
            }
        });
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "left" => {
                self.cursor = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i)
            }
            "right" => {
                if let Some(ch) = self.value[self.cursor..].chars().next() {
                    self.cursor += ch.len_utf8();
                }
            }
            "home" => self.cursor = 0,
            "end" => self.cursor = self.value.len(),
            "backspace" => {
                if let Some(i) = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                {
                    self.value.replace_range(i..self.cursor, "");
                    self.cursor = i;
                }
            }
            "delete" => {
                if let Some(ch) = self.value[self.cursor..].chars().next() {
                    self.value
                        .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
                }
            }
            _ if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.alt =>
            {
                let Some(text) = &event.keystroke.key_char else {
                    return;
                };
                if matches!(self.field, Field::Color)
                    && (!text.chars().all(|ch| ch == '#' || ch.is_ascii_hexdigit())
                        || self.value.len() + text.len() > 7)
                {
                    return;
                }
                self.value.insert_str(self.cursor, text);
                self.cursor += text.len();
            }
            _ => return,
        }
        self.publish(cx);
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|input, _: &MouseDownEvent, window, cx| {
                    input.focus.focus(window);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(Self::key_down))
            .h(px(38.0))
            .w_full()
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border(px(1.0))
            .border_color(rgb(if self.focus.is_focused(window) {
                0x6d5dfc
            } else {
                0xcbd5e1
            }))
            .bg(rgb(0xffffff))
            .cursor_text()
            .child(self.value.clone())
    }
}

struct Example {
    graph: Entity<Graph>,
    color: Entity<TextInput>,
    label: Entity<TextInput>,
    hidden: bool,
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = div()
            .absolute()
            .left(px(20.0))
            .top(px(20.0))
            .w(px(260.0))
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(9.0))
            .rounded(px(10.0))
            .border(px(1.0))
            .border_color(rgb(0xe2e8f0))
            .bg(rgb(0xfffffff0))
            .shadow_sm()
            .child(div().text_size(px(13.0)).child("Node 1 color (hex)"))
            .child(self.color.clone())
            .child(div().mt(px(4.0)).text_size(px(13.0)).child("Node 1 label"))
            .child(self.label.clone())
            .child(
                div()
                    .mt(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseDownEvent, _, cx| {
                            view.hidden = !view.hidden;
                            let hidden = view.hidden;
                            update_node(&view.graph, cx, move |node| node.hidden = hidden);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .size(px(20.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .border(px(2.0))
                            .border_color(rgb(0x6d5dfc))
                            .bg(rgb(if self.hidden { 0x6d5dfc } else { 0xffffff }))
                            .text_color(rgb(0xffffff))
                            .child(if self.hidden { "✓" } else { "" }),
                    )
                    .child("Hidden"),
            );
        div()
            .relative()
            .size_full()
            .bg(rgb(0xf8fafc))
            .child(self.graph.clone())
            .child(controls)
    }
}

fn make_node(id: u64, x: f32, label: &str, color: &str) -> Node {
    let mut node = Node::new(id, WorldPoint::new(x, 0.0))
        .with_size(WorldSize::new(18.0, 9.0))
        .with_type(EDITABLE_NODE);
    node.metadata.insert("caption".into(), label.into());
    node.metadata.insert("color".into(), color.into());
    node
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Updating Nodes".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type(
                    EDITABLE_NODE,
                    |node: &Node, _: f32, _: &gpug::GraphStyle| NodeAppearance {
                        color: node
                            .metadata
                            .get("color")
                            .and_then(|value| parse_color(value))
                            .unwrap_or(0x6d5dfc),
                        radius_pixels: 8.0,
                        shape: NodeShape::Rect {
                            corner_radius_world: 1.5,
                            border_color: 0x4338ca,
                            border_width_pixels: 1.0,
                        },
                    },
                );
                renderer.register_cached_node_content(EDITABLE_NODE, |node: &Node, zoom: f32| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0xffffff))
                        .text_size(px((1.9 * zoom).clamp(13.0, 22.0)))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(
                            vec![
                                make_node(1, -12.0, "Node 1", "#6d5dfc"),
                                make_node(2, 12.0, "Node 2", "#0f9d8a"),
                            ],
                            vec![Edge::new_with_id(1_u64, 2_u64, 1_u64)],
                        ))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                let color = cx.new(|cx| TextInput::new(graph.clone(), Field::Color, "#6d5dfc", cx));
                let label = cx.new(|cx| TextInput::new(graph.clone(), Field::Label, "Node 1", cx));
                cx.new(|_| Example {
                    graph,
                    color,
                    label,
                    hidden: false,
                })
            },
        )
        .unwrap();
    });
}

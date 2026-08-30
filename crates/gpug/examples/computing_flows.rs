use gpug::{
    Edge, EdgeMarker, Graph, GraphData, GraphDataApi, GraphRenderer, HandleKind, Node,
    NodeAppearance, NodeId, NodeShape, WorldPoint, WorldSize,
};
use gpui::{
    div, fill, point, px, rgb, size, App, AppContext, Application, Bounds, ContentMask, Context,
    Element, ElementId, Entity, FocusHandle, Focusable, GlobalElementId, InteractiveElement,
    IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, PaintQuad, ParentElement,
    Pixels, Render, ShapedLine, SharedString, Style, Styled, TextRun, Window, WindowOptions,
};

const NODE_ONE: NodeId = NodeId(1);
const NODE_TWO: NodeId = NodeId(2);
const UPPERCASE: NodeId = NodeId(3);
const OUTPUT: NodeId = NodeId(4);

fn is_text_node(node: &Node) -> bool {
    matches!(
        node.node_type.as_str(),
        "text_input" | "uppercase" | "output_values" | "substring_match"
    )
}

fn matching_substrings(text: &str) -> String {
    let lowercase = text.to_lowercase();
    let mut hits = ["gpug", "world"]
        .into_iter()
        .flat_map(|needle| {
            lowercase
                .match_indices(needle)
                .map(move |(index, _)| (index, needle))
        })
        .collect::<Vec<_>>();
    hits.sort_by_key(|(index, _)| *index);
    hits.into_iter()
        .map(|(_, hit)| hit)
        .collect::<Vec<_>>()
        .join(" ")
}

struct FlowInput {
    node: NodeId,
    value: String,
    focus: FocusHandle,
    data_api: GraphDataApi,
    cursor: usize,
    scroll_x: Pixels,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
}

impl FlowInput {
    fn new(node: NodeId, value: &str, data_api: GraphDataApi, cx: &mut App) -> Self {
        Self {
            node,
            value: value.into(),
            focus: cx.focus_handle().tab_stop(true),
            data_api,
            cursor: value.len(),
            scroll_x: px(0.0),
            last_layout: None,
            last_bounds: None,
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window);
        if let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) {
            self.cursor =
                line.closest_index_for_x(event.position.x - bounds.left() + self.scroll_x);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
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
                        .replace_range(self.cursor..self.cursor + next.len_utf8(), "");
                }
            }
            "left" => {
                self.cursor = self.value[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
            "right" => {
                if let Some(next) = self.value[self.cursor..].chars().next() {
                    self.cursor += next.len_utf8();
                }
            }
            "home" => self.cursor = 0,
            "end" => self.cursor = self.value.len(),
            _ if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.alt =>
            {
                if let Some(text) = &event.keystroke.key_char {
                    self.value.insert_str(self.cursor, text);
                    self.cursor += text.len();
                } else {
                    return;
                }
            }
            _ => return,
        }
        self.data_api
            .update_node_data(self.node, [("text".into(), self.value.clone())]);
        cx.stop_propagation();
        cx.refresh_windows();
        cx.notify();
    }
}

impl Focusable for FlowInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FlowInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        div()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .h_full()
            .px_3()
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border(px(1.0))
            .border_color(rgb(if focused { 0xff0072 } else { 0xcbd5e1 }))
            .bg(rgb(0xffffff))
            .cursor_text()
            .overflow_hidden()
            .child(FlowTextElement { input: cx.entity() })
    }
}

struct FlowTextElement {
    input: Entity<FlowInput>,
}

struct FlowTextPrepaint {
    line: ShapedLine,
    cursor: PaintQuad,
    scroll_x: Pixels,
}

impl IntoElement for FlowTextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FlowTextElement {
    type RequestLayoutState = ();
    type PrepaintState = FlowTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> FlowTextPrepaint {
        let input = self.input.read(cx);
        let text = if input.value.is_empty() {
            SharedString::from("Type a value…")
        } else {
            SharedString::from(input.value.clone())
        };
        let text_style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: text_style.font(),
            color: text_style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            text,
            text_style.font_size.to_pixels(window.rem_size()),
            &[run],
            None,
        );
        let cursor_x = line.x_for_index(input.cursor);
        let width = bounds.size.width.max(px(0.0));
        let max_scroll = (line.width - width).max(px(0.0));
        let mut scroll_x = input.scroll_x.min(max_scroll);
        if cursor_x < scroll_x {
            scroll_x = cursor_x;
        } else if cursor_x > scroll_x + width - px(2.0) {
            scroll_x = (cursor_x - width + px(2.0)).min(max_scroll);
        }
        FlowTextPrepaint {
            line,
            cursor: fill(
                Bounds::new(
                    point(bounds.left() + cursor_x - scroll_x, bounds.top()),
                    size(px(1.0), bounds.size.height),
                ),
                rgb(0x0f172a),
            ),
            scroll_x,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut FlowTextPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            prepaint
                .line
                .paint(
                    point(bounds.left() - prepaint.scroll_x, bounds.top()),
                    bounds.size.height,
                    window,
                    cx,
                )
                .unwrap();
            if focus.is_focused(window) {
                window.paint_quad(prepaint.cursor.clone());
            }
        });
        self.input.update(cx, |input, _| {
            input.scroll_x = prepaint.scroll_x;
            input.last_layout = Some(prepaint.line.clone());
            input.last_bounds = Some(bounds);
        });
    }
}

fn card(title: &str, zoom: f32, body: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .p(px(0.8 * zoom))
        .flex()
        .flex_col()
        .gap(px(0.55 * zoom))
        .text_size(px(1.65 * zoom))
        .rounded(px(0.8 * zoom))
        .border(px(1.0))
        .border_color(rgb(0xcbd5e1))
        .bg(rgb(0xffffff))
        .shadow_sm()
        .overflow_hidden()
        .child(div().text_color(rgb(0x475569)).child(title.to_string()))
        .child(div().flex_1().overflow_hidden().child(body))
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Computing Flows".into()),
                ..Default::default()
            },
            |_, cx| {
                let data_api = GraphDataApi::new();
                let input_one =
                    cx.new(|cx| FlowInput::new(NODE_ONE, "hello", data_api.clone(), cx));
                let mut nodes = vec![
                    Node::new(NODE_ONE, WorldPoint::new(0.0, 0.0))
                        .with_size(WorldSize::new(18.0, 9.0))
                        .with_type("text_input"),
                    Node::new(UPPERCASE, WorldPoint::new(30.0, 0.0))
                        .with_size(WorldSize::new(18.0, 8.0))
                        .with_type("uppercase"),
                    Node::new(OUTPUT, WorldPoint::new(57.0, 0.0))
                        .with_size(WorldSize::new(20.0, 13.0))
                        .with_type("output_values"),
                    Node::new(NODE_TWO, WorldPoint::new(86.0, 0.0))
                        .with_size(WorldSize::new(18.0, 13.0))
                        .with_type("substring_match"),
                ];
                nodes[0].metadata.insert("text".into(), "hello".into());
                nodes[1].metadata.insert("text".into(), String::new());
                nodes[2].metadata.insert("text".into(), String::new());
                nodes[3].metadata.insert("text".into(), String::new());
                let mut edges = vec![
                    Edge::new(NODE_ONE, UPPERCASE).with_id(1_u64),
                    Edge::new(UPPERCASE, OUTPUT).with_id(2_u64),
                    Edge::new(OUTPUT, NODE_TWO).with_id(3_u64),
                ];
                for edge in &mut edges {
                    edge.edge_type = "bezier".into();
                    edge.marker_end = Some(EdgeMarker::ArrowClosed);
                }

                let mut renderer = GraphRenderer::default();
                for kind in [
                    "text_input",
                    "uppercase",
                    "output_values",
                    "substring_match",
                ] {
                    renderer.register_node_type(kind, |node: &Node, zoom, _: &gpug::GraphStyle| {
                        NodeAppearance {
                            color: 0xffffff,
                            radius_pixels: node.size.width * zoom * 0.5,
                            shape: NodeShape::None,
                        }
                    });
                }
                let first = input_one.clone();
                renderer.register_node_content("text_input", move |node: &Node, zoom| {
                    let input: Entity<FlowInput> = first.clone();
                    card(&format!("node {}", node.id.0), zoom, input).into_any_element()
                });
                let matcher_api = data_api.clone();
                renderer.register_node_content("substring_match", move |node: &Node, zoom| {
                    let incoming = matcher_api
                        .node_connections(node.id, HandleKind::Target)
                        .first()
                        .and_then(|connection| matcher_api.node_data(connection.source))
                        .and_then(|source| source.metadata.get("text").cloned())
                        .unwrap_or_default();
                    let value = matching_substrings(&incoming);
                    matcher_api.update_node_data(node.id, [("text".into(), value.clone())]);
                    card(
                        &format!("node {} · matches gpug/world", node.id.0),
                        zoom,
                        div()
                            .w_full()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(value),
                    )
                    .into_any_element()
                });
                let transform_api = data_api.clone();
                renderer.register_node_content("uppercase", move |node: &Node, zoom| {
                    let source = transform_api
                        .node_connections(node.id, HandleKind::Target)
                        .first()
                        .map(|connection| connection.source);
                    let value = source
                        .and_then(|source| transform_api.node_data(source))
                        .filter(is_text_node)
                        .and_then(|node| node.metadata.get("text").cloned())
                        .unwrap_or_default()
                        .to_uppercase();
                    transform_api.update_node_data(node.id, [("text".into(), value.clone())]);
                    card(
                        "uppercase transform",
                        zoom,
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(value),
                    )
                    .into_any_element()
                });
                let output_api = data_api.clone();
                renderer.register_node_content("output_values", move |node: &Node, zoom| {
                    let value = output_api
                        .node_connections(node.id, HandleKind::Target)
                        .first()
                        .and_then(|connection| output_api.node_data(connection.source))
                        .filter(is_text_node)
                        .and_then(|source| source.metadata.get("text").cloned())
                        .unwrap_or_default();
                    output_api.update_node_data(node.id, [("text".into(), value.clone())]);
                    card(
                        "incoming texts",
                        zoom,
                        div().flex().flex_col().gap_1().children(
                            (!value.is_empty())
                                .then_some(value)
                                .into_iter()
                                .map(|value| {
                                    div()
                                        .w_full()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(value)
                                }),
                        ),
                    )
                    .into_any_element()
                });

                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, edges))
                        .data_api(data_api)
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| ComputingFlows { graph, input_one })
            },
        )
        .unwrap();
    });
}

struct ComputingFlows {
    graph: Entity<Graph>,
    input_one: Entity<FlowInput>,
}

impl Render for ComputingFlows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.input_one;
        div().size_full().child(self.graph.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_producers_are_exposed_to_transforms_and_results() {
        assert!(is_text_node(
            &Node::new(1_u64, WorldPoint::ZERO).with_type("text_input")
        ));
        assert!(is_text_node(
            &Node::new(2_u64, WorldPoint::ZERO).with_type("uppercase")
        ));
        assert!(is_text_node(
            &Node::new(3_u64, WorldPoint::ZERO).with_type("output_values")
        ));
    }

    #[test]
    fn substring_match_combines_all_hits_in_source_order() {
        assert_eq!(
            matching_substrings("WORLD says gpug, then world and GPUG"),
            "world gpug world gpug"
        );
        assert_eq!(matching_substrings("no matches"), "");
    }
}

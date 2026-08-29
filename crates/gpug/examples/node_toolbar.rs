use gpug::{
    Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeShape, Position, WorldPoint,
    WorldSize,
};
use gpui::{
    canvas, div, prelude::*, px, rgb, App, AppContext, Application, Context, Entity, MouseButton,
    MouseDownEvent, Render, Window, WindowOptions,
};

const TOOLBAR_NODE: &str = "toolbar-node";
const PINK: u32 = 0xff0072;

#[derive(Clone, Copy, PartialEq)]
enum Alignment {
    Start,
    Center,
    End,
}

struct NodeToolbarExample {
    graph: Entity<Graph>,
    position: Position,
    alignment: Alignment,
    always_visible: bool,
}

impl NodeToolbarExample {
    fn button(
        &self,
        label: &'static str,
        active: bool,
        handler: impl Fn(&mut Self) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .px(px(16.0))
            .h(px(40.0))
            .flex()
            .items_center()
            .rounded_full()
            .border(px(1.0))
            .border_color(rgb(PINK))
            .bg(rgb(if active { 0xffe4f0 } else { 0xffffff }))
            .text_color(rgb(PINK))
            .text_size(px(14.0))
            .shadow_sm()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                    handler(view);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(label)
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (top_left, bottom_right) = cx.read_entity(&self.graph, |graph, _| {
            let node = graph
                .selected_nodes()
                .next()
                .or_else(|| self.always_visible.then(|| graph.nodes().first()).flatten())?;
            let bounds = graph.node_bounds(node.id)?;
            let viewport = graph.viewport();
            Some((
                viewport.world_to_screen(bounds.origin),
                viewport.world_to_screen(WorldPoint::new(
                    bounds.origin.x + bounds.size.width,
                    bounds.origin.y + bounds.size.height,
                )),
            ))
        })?;
        let (node_width, node_height) = (bottom_right.x - top_left.x, bottom_right.y - top_left.y);
        let (toolbar_width, toolbar_height, gap) = (px(184.0), px(42.0), px(10.0));
        let along_x = match self.alignment {
            Alignment::Start => top_left.x,
            Alignment::Center => top_left.x + (node_width - toolbar_width) / 2.0,
            Alignment::End => bottom_right.x - toolbar_width,
        };
        let along_y = match self.alignment {
            Alignment::Start => top_left.y,
            Alignment::Center => top_left.y + (node_height - toolbar_height) / 2.0,
            Alignment::End => bottom_right.y - toolbar_height,
        };
        let (left, top) = match self.position {
            Position::Top => (along_x, top_left.y - toolbar_height - gap),
            Position::Bottom => (along_x, bottom_right.y + gap),
            Position::Left => (top_left.x - toolbar_width - gap, along_y),
            Position::Right => (bottom_right.x + gap, along_y),
        };
        Some(
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(toolbar_width)
                .h(toolbar_height)
                .flex()
                .gap(px(6.0))
                .items_center()
                .justify_center()
                .children(["cut", "copy", "paste"].map(|label| {
                    div()
                        .px(px(14.0))
                        .h(px(40.0))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .border(px(1.0))
                        .border_color(rgb(PINK))
                        .bg(rgb(0xffffff))
                        .text_color(rgb(PINK))
                        .text_size(px(14.0))
                        .shadow_sm()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(label)
                })),
        )
    }
}

impl Render for NodeToolbarExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let repaint = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                cx.update_entity(&view, |_, cx| cx.notify());
            },
        )
        .absolute()
        .size(px(1.0));
        let controls = div()
            .absolute()
            .left(px(14.0))
            .top(px(28.0))
            .w(px(310.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .cursor_default()
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Node Toolbar position:"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(self.button(
                        "top",
                        self.position == Position::Top,
                        |v| v.position = Position::Top,
                        cx,
                    ))
                    .child(self.button(
                        "right",
                        self.position == Position::Right,
                        |v| v.position = Position::Right,
                        cx,
                    ))
                    .child(self.button(
                        "bottom",
                        self.position == Position::Bottom,
                        |v| v.position = Position::Bottom,
                        cx,
                    ))
                    .child(self.button(
                        "left",
                        self.position == Position::Left,
                        |v| v.position = Position::Left,
                        cx,
                    )),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .text_size(px(20.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Node Toolbar Alignment:"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(self.button(
                        "start",
                        self.alignment == Alignment::Start,
                        |v| v.alignment = Alignment::Start,
                        cx,
                    ))
                    .child(self.button(
                        "center",
                        self.alignment == Alignment::Center,
                        |v| v.alignment = Alignment::Center,
                        cx,
                    ))
                    .child(self.button(
                        "end",
                        self.alignment == Alignment::End,
                        |v| v.alignment = Alignment::End,
                        cx,
                    )),
            )
            .child(
                div()
                    .mt(px(2.0))
                    .text_size(px(20.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Override Node Toolbar visibility"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseDownEvent, _, cx| {
                            view.always_visible = !view.always_visible;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .size(px(20.0))
                            .rounded(px(6.0))
                            .border(px(2.0))
                            .border_color(rgb(PINK))
                            .bg(rgb(if self.always_visible { PINK } else { 0xffffff })),
                    )
                    .child(div().text_size(px(16.0)).child("Always show toolbar")),
            );
        let mut root = div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(repaint)
            .child(controls);
        if let Some(toolbar) = self.toolbar(cx) {
            root = root.child(toolbar);
        }
        root
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Node Toolbar".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut node = Node::new(1_u64, WorldPoint::new(4.0, 0.0))
                    .with_size(WorldSize::new(38.0, 8.0))
                    .with_type(TOOLBAR_NODE);
                node.metadata
                    .insert("caption".into(), "Select me to show the toolbar".into());
                let mut renderer = GraphRenderer::default();
                let mut style = renderer.style().clone();
                style.selection_color = PINK;
                renderer.set_style(style);
                renderer.register_node_type(
                    TOOLBAR_NODE,
                    |node: &Node, zoom: f32, _: &gpug::GraphStyle| NodeAppearance {
                        color: 0xffffff,
                        radius_pixels: node.size.width * zoom * 0.5,
                        shape: NodeShape::None,
                    },
                );
                renderer.register_node_content(TOOLBAR_NODE, |node: &Node, zoom: f32| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.0))
                        .border(px(2.0))
                        .border_color(rgb(if node.selected { PINK } else { 0xff69b4 }))
                        .bg(rgb(0xffffff))
                        .shadow_md()
                        .text_size(px((2.5 * zoom).clamp(18.0, 26.0)))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(vec![node], vec![]))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| NodeToolbarExample {
                    graph,
                    position: Position::Top,
                    alignment: Alignment::Center,
                    always_visible: false,
                })
            },
        )
        .unwrap();
    });
}

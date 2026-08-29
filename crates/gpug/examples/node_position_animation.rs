use std::time::{Duration, Instant};

use gpug::{
    Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeId, NodeShape, WorldPoint, WorldSize,
};
use gpui::{
    App, AppContext, Application, Context, Entity, MouseButton, MouseDownEvent, Render, Window,
    WindowOptions, canvas, div, prelude::*, px, rgb,
};

const NODE_COUNT: usize = 5;
const ANIMATION_DURATION: Duration = Duration::from_millis(700);
const PINK: u32 = 0xff0072;

fn horizontal_layout() -> [WorldPoint; NODE_COUNT] {
    std::array::from_fn(|index| WorldPoint::new(index as f32 * 20.0, 24.0))
}

fn vertical_layout() -> [WorldPoint; NODE_COUNT] {
    std::array::from_fn(|index| WorldPoint::new(40.0, index as f32 * 12.0))
}

fn lerp(from: WorldPoint, to: WorldPoint, amount: f32) -> WorldPoint {
    WorldPoint::new(
        from.x + (to.x - from.x) * amount,
        from.y + (to.y - from.y) * amount,
    )
}

struct PositionAnimation {
    graph: Entity<Graph>,
    current: [WorldPoint; NODE_COUNT],
    from: [WorldPoint; NODE_COUNT],
    to: [WorldPoint; NODE_COUNT],
    started_at: Option<Instant>,
    vertical: bool,
}

impl PositionAnimation {
    fn toggle_layout(&mut self) {
        self.vertical = !self.vertical;
        self.from = self.current;
        self.to = if self.vertical {
            vertical_layout()
        } else {
            horizontal_layout()
        };
        self.started_at = Some(Instant::now());
    }

    fn advance(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(started_at) = self.started_at else {
            return;
        };
        let amount = (now.duration_since(started_at).as_secs_f32()
            / ANIMATION_DURATION.as_secs_f32())
        .min(1.0);

        self.current = std::array::from_fn(|index| lerp(self.from[index], self.to[index], amount));
        cx.update_entity(&self.graph, |graph, cx| {
            for (index, position) in self.current.iter().copied().enumerate() {
                graph
                    .set_node_position(NodeId((index + 1) as u64), position)
                    .expect("the example graph remains valid");
            }
            cx.notify();
        });

        if amount == 1.0 {
            self.started_at = None;
        } else {
            cx.notify();
        }
    }
}

impl Render for PositionAnimation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let animation_tick = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                let animating = cx.read_entity(&view, |view, _| view.started_at.is_some());
                if animating {
                    window.request_animation_frame();
                    cx.update_entity(&view, |view, cx| view.advance(Instant::now(), cx));
                }
            },
        )
        .absolute()
        .size(px(1.0));

        let toggle = div()
            .absolute()
            .left(px(15.0))
            .top(px(18.0))
            .h(px(40.0))
            .px(px(16.0))
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| {
                    view.toggle_layout();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child("toggle layout");

        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(animation_tick)
            .child(toggle)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Node Position Animation".into()),
                ..Default::default()
            },
            |_, cx| {
                let positions = horizontal_layout();
                let nodes = positions
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, position)| {
                        let mut node = Node::new((index + 1) as u64, position)
                            .with_size(WorldSize::new(14.0, 7.0))
                            .with_type("position-card");
                        node.draggable = false;
                        node.metadata
                            .insert("caption".into(), ((b'A' + index as u8) as char).to_string());
                        node
                    })
                    .collect();

                let mut renderer = GraphRenderer::default();
                renderer.register_node_type(
                    "position-card",
                    |_: &Node, _: f32, _: &gpug::GraphStyle| NodeAppearance {
                        color: 0xffffff,
                        radius_pixels: 0.0,
                        shape: NodeShape::None,
                    },
                );
                renderer.register_node_content("position-card", |node: &Node, zoom: f32| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.0))
                        .border(px(1.0))
                        .border_color(rgb(0xe4e4e7))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px((1.8 * zoom).clamp(10.0, 16.0)))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });

                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, vec![]))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| PositionAnimation {
                    graph,
                    current: positions,
                    from: positions,
                    to: positions,
                    started_at: None,
                    vertical: false,
                })
            },
        )
        .unwrap();
    });
}

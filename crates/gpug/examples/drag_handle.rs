use gpug::{
    Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeShape, WorldBounds, WorldPoint,
    WorldSize,
};
use gpui::{
    div, px, rgb, App, AppContext, Application, IntoElement, ParentElement, Styled, WindowOptions,
};

const NODE_SIZE: WorldSize = WorldSize::new(46.0, 12.0);
const DRAG_HANDLE: WorldBounds =
    WorldBounds::new(WorldPoint::new(37.0, 2.0), WorldSize::new(8.0, 8.0));
const NO_DRAG: WorldBounds = WorldBounds::new(WorldPoint::new(39.5, 4.5), WorldSize::new(3.0, 3.0));

fn transparent_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Drag Handle".into()),
                ..Default::default()
            },
            |_, cx| {
                let node = Node::new(1_u64, WorldPoint::new(0.0, 0.0))
                    .with_size(NODE_SIZE)
                    .with_type("drag-handle")
                    .with_custom_handle(DRAG_HANDLE)
                    // A no-drag child wins even when it is inside the handle.
                    .with_nodrag(NO_DRAG);

                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("drag-handle", transparent_node);
                renderer.register_node_content("drag-handle", |_: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .px(px(2.0 * zoom))
                        .rounded(px(2.0 * zoom))
                        .border(px(1.5))
                        .border_color(rgb(0xff5eae))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px(2.35 * zoom))
                        .child(div().flex_1().child("Only draggable by the ring →"))
                        .child(
                            div()
                                .size(px(8.0 * zoom))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rgb(0x078c8c))
                                .child(div().size(px(3.0 * zoom)).rounded_full().bg(rgb(0xff5eae))),
                        )
                        .into_any_element()
                });

                cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(vec![node], vec![]))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
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
    fn example_declares_a_nested_nodrag_region() {
        assert!(NO_DRAG.origin.x >= DRAG_HANDLE.origin.x);
        assert!(NO_DRAG.origin.y >= DRAG_HANDLE.origin.y);
        assert!(
            NO_DRAG.origin.x + NO_DRAG.size.width <= DRAG_HANDLE.origin.x + DRAG_HANDLE.size.width
        );
        assert!(
            NO_DRAG.origin.y + NO_DRAG.size.height
                <= DRAG_HANDLE.origin.y + DRAG_HANDLE.size.height
        );
    }
}

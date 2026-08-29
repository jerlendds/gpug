use gpug::{
    Edge, Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeShape, WorldBounds,
    WorldPoint, WorldSize,
};
use gpui::{
    div, px, rgb, App, AppContext, Application, IntoElement, ParentElement, Styled, WindowOptions,
};

const NODE_SIZE: WorldSize = WorldSize::new(18.0, 10.0);
const DRAG_HANDLE: WorldBounds =
    WorldBounds::new(WorldPoint::new(0.0, 0.0), WorldSize::new(18.0, 2.0));

fn invisible_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

fn example_data() -> GraphData {
    let make_node = |id, position, caption: &str| {
        let mut node = Node::new(id, position)
            .with_size(NODE_SIZE)
            .with_type("easy-connect")
            .with_connectable_body()
            .with_custom_handle(DRAG_HANDLE);
        node.metadata.insert("caption".into(), caption.into());
        node
    };
    GraphData::new(
        vec![
            make_node(1_u64, WorldPoint::new(0.0, 0.0), "Drop here"),
            make_node(2_u64, WorldPoint::new(30.0, 0.0), "Drop here"),
            make_node(3_u64, WorldPoint::new(4.0, 22.0), "Drop here"),
            make_node(4_u64, WorldPoint::new(26.0, 22.0), "Drag to connect"),
        ],
        vec![
            Edge::new(2_u64, 1_u64),
            Edge::new(1_u64, 3_u64),
            Edge::new(4_u64, 1_u64),
        ],
    )
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Easy Connect".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("easy-connect", invisible_node);
                renderer.register_node_content("easy-connect", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(1.0 * zoom))
                        .border(px(1.0))
                        .border_color(rgb(if node.id == 4_u64.into() {
                            0xff5eae
                        } else {
                            0xe2e4e7
                        }))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px(1.5 * zoom))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });
                cx.new(|cx| {
                    Graph::builder()
                        .data(example_data())
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
    fn every_node_has_a_body_port_and_a_separate_drag_handle() {
        let data = example_data();
        assert_eq!(data.nodes.len(), 4);
        assert!(data.nodes.iter().all(|node| node.connectable_body));
        assert!(data
            .nodes
            .iter()
            .all(|node| node.custom_handle == Some(DRAG_HANDLE)));
    }
}

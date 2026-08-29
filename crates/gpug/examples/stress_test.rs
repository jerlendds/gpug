use gpug::{
    Edge, Graph, GraphData, GraphRenderer, GraphStyle, Node, NodeAppearance, NodeShape, Position,
    WorldPoint, WorldSize,
};
use gpui::{
    div, px, rgb, App, AppContext, Application, IntoElement, ParentElement, Styled, WindowOptions,
};

const NODE_COUNT: usize = 1_000;
const COLUMNS: usize = 20;
const ROWS: usize = NODE_COUNT / COLUMNS;
const NODE_SIZE: WorldSize = WorldSize::new(11.0, 7.0);

fn stress_node(_: &Node, _: f32, _: &GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: NODE_SIZE.width * 0.5,
        shape: NodeShape::None,
    }
}

fn stress_data() -> GraphData {
    let mut nodes = Vec::with_capacity(NODE_COUNT);
    let mut edges = Vec::with_capacity(NODE_COUNT);
    for index in 0..NODE_COUNT {
        let column = index / ROWS;
        let row = index % ROWS;
        let id = index as u64 + 1;
        let mut node = Node::new(id, WorldPoint::new(column as f32 * 18.0, row as f32 * 15.0))
            .with_size(NODE_SIZE)
            .with_type("stress");
        node.metadata
            .insert("caption".into(), format!("Node\n{id}"));
        nodes.push(node);

        // One cycle gives every node exactly one incoming and outgoing edge.
        let target = (index + 1) % NODE_COUNT + 1;
        edges.push(Edge::new_with_id(id, target as u64, id));
    }
    GraphData::new(nodes, edges)
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Stress Test".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("stress", stress_node);
                renderer.register_node_content("stress", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(1.6 * zoom))
                        .border(px(1.0))
                        .border_color(rgb(if node.selected { 0xff5eae } else { 0xd4d4d8 }))
                        .bg(rgb(0xffffff))
                        .text_size(px(2.0 * zoom))
                        .child(node.metadata["caption"].clone())
                        .into_any_element()
                });
                cx.new(|cx| {
                    Graph::builder()
                        .data(stress_data())
                        .renderer(renderer)
                        .handle_positions(Position::Top, Position::Bottom)
                        .show_handles(true)
                        .only_render_visible_elements(true)
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
    fn every_node_has_one_incoming_and_outgoing_edge() {
        let data = stress_data();
        assert_eq!(
            (data.nodes.len(), data.edges.len()),
            (NODE_COUNT, NODE_COUNT)
        );
        for node in &data.nodes {
            assert_eq!(
                data.edges
                    .iter()
                    .filter(|edge| edge.source == node.id)
                    .count(),
                1
            );
            assert_eq!(
                data.edges
                    .iter()
                    .filter(|edge| edge.target == node.id)
                    .count(),
                1
            );
            assert!(node.draggable);
        }
    }
}

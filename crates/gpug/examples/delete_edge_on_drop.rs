mod support;

use gpug::{
    ConnectionIntent, Edge, Graph, GraphData, GraphEvent, GraphRenderer, Node, NodeAppearance,
    NodeShape, Position, WorldPoint, WorldSize,
};
use gpui::{div, prelude::*, px, rgb, Context};

const CARD: &str = "delete-edge-card";

fn data() -> GraphData {
    let card = |id, position, caption: &str| {
        let mut node = Node::new(id, position)
            .with_size(WorldSize::new(18.0, 7.0))
            .with_type(CARD);
        node.metadata.insert("caption".into(), caption.into());
        node
    };
    GraphData::new(
        vec![
            card(1_u64, WorldPoint::new(0.0, -16.0), "Node A"),
            card(2_u64, WorldPoint::new(-18.0, 14.0), "Node B"),
            card(3_u64, WorldPoint::new(18.0, 14.0), "Node C"),
        ],
        vec![
            Edge::new_with_id(1_u64, 2_u64, 1_u64).with_marker_end(None),
            Edge::new_with_id(1_u64, 3_u64, 2_u64).with_marker_end(None),
        ],
    )
}

fn renderer(renderer: &mut GraphRenderer) {
    let mut style = renderer.style().clone();
    style.selection_color = 0xff0072;
    style.edge_color = 0xb1b1b7;
    style.edge_width_pixels = 2.0;
    renderer.set_style(style);
    renderer.register_node_type(CARD, |_: &Node, _: f32, _: &gpug::GraphStyle| {
        NodeAppearance {
            color: 0xffffff,
            radius_pixels: 0.0,
            shape: NodeShape::None,
        }
    });
    renderer.register_node_content(CARD, |node: &Node, zoom: f32| {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(1.0 * zoom))
            .border(px(1.0))
            .border_color(rgb(if node.selected { 0xff0072 } else { 0xe3e3e3 }))
            .bg(rgb(0xffffff))
            .shadow_md()
            .text_color(rgb(0x111111))
            .text_size(px((1.6 * zoom).clamp(13.0, 18.0)))
            .child(node.metadata.get("caption").cloned().unwrap_or_default())
            .into_any_element()
    });
}

fn delete_on_failed_reconnect(graph: &mut Graph, event: &GraphEvent, cx: &mut Context<Graph>) {
    let GraphEvent::ConnectEnd {
        intent,
        connected: false,
        ..
    } = event
    else {
        return;
    };
    let edge = match intent {
        ConnectionIntent::ReconnectSource(id) | ConnectionIntent::ReconnectTarget(id) => *id,
        ConnectionIntent::Create => return,
    };
    if graph.select_edge(edge) && graph.delete_selected() {
        cx.notify();
    }
}

catalog_example!(support::CatalogExample::new(
    "Delete Edge on Drop",
    "Reconnect an endpoint and drop it on the pane to delete that edge."
)
.initial_data(data)
.configure_renderer(renderer)
.show_handles(true)
.handle_positions(Position::Top, Position::Bottom)
.markerless_created_edges()
.show_zoom(true)
.on_event(delete_on_failed_reconnect));

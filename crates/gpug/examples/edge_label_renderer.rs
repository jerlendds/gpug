mod support;

use gpug::{
    Edge, EdgeLabelContext, GraphData, GraphRenderer, GraphStyle, Node, NodeAppearance, NodeShape,
    Position, WorldPoint, WorldSize,
};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement};

fn node(id: u64, x: f32, y: f32, caption: &str) -> Node {
    let mut node = Node::new(id, WorldPoint::new(x, y)).with_size(WorldSize::new(17.0, 4.5));
    node.metadata.insert("caption".into(), caption.into());
    node
}

fn data() -> GraphData {
    let nodes = vec![
        node(1, -21.0, -15.0, "Node 1"),
        node(2, -21.0, 15.0, "Node 2"),
        node(3, 9.0, -15.0, "Node 3"),
        node(4, 9.0, 15.0, "Node 4"),
    ];
    let mut center = Edge::new_with_id(1_u64, 2_u64, 1_u64);
    center.marker_end = None;
    center.label = Some("edge label".into());
    center.metadata.insert("labels".into(), "center".into());
    let mut ends = Edge::new_with_id(3_u64, 4_u64, 2_u64);
    ends.marker_end = None;
    ends.label = Some("edge labels".into());
    ends.metadata.insert("labels".into(), "ends".into());
    GraphData::new(nodes, vec![center, ends])
}

fn label(
    text: &str,
    center_x: gpui::Pixels,
    top: gpui::Pixels,
    width_world: f32,
    accent: bool,
    zoom: f32,
) -> AnyElement {
    let width = px(width_world * zoom);
    div()
        .absolute()
        .left(center_x - width * 0.5)
        .top(top)
        .w(width)
        .h(px(3.6 * zoom))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(0.5 * zoom))
        .border(px((0.08 * zoom).max(0.5)))
        .border_color(if accent { rgb(0xd9dde5) } else { rgb(0xffcc00) })
        .bg(if accent {
            rgba(0xfffffff2)
        } else {
            rgb(0xffd400)
        })
        .text_color(if accent { rgb(0xff4d4f) } else { rgb(0x111111) })
        .text_size(px(1.25 * zoom))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.to_string())
        .into_any_element()
}

fn configure_renderer(renderer: &mut GraphRenderer) {
    renderer.register_node_type("default", |_: &Node, _: f32, _: &GraphStyle| {
        NodeAppearance {
            color: 0xffffff,
            radius_pixels: 0.0,
            shape: NodeShape::Rect {
                corner_radius_world: 0.8,
                border_color: 0xd9dde5,
                border_width_pixels: 1.0,
            },
        }
    });
    renderer.register_cached_node_content("default", |node: &Node, _| {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(node.metadata.get("caption").cloned().unwrap_or_default())
            .into_any_element()
    });
    renderer.set_edge_label_renderer(|edge: &Edge, anchors: EdgeLabelContext| {
        let zoom = anchors.zoom.max(0.0);
        if edge.metadata.get("labels").map(String::as_str) == Some("ends") {
            div()
                .children([
                    label(
                        "Start edge label",
                        anchors.source.x,
                        anchors.source.y + px(1.2 * zoom),
                        12.0,
                        true,
                        zoom,
                    ),
                    label(
                        "End edge label",
                        anchors.target.x,
                        anchors.target.y - px(4.8 * zoom),
                        12.0,
                        true,
                        zoom,
                    ),
                ])
                .into_any_element()
        } else {
            label(
                "edge label",
                anchors.midpoint.x,
                anchors.midpoint.y - px(1.8 * zoom),
                8.5,
                false,
                zoom,
            )
        }
    });
}

catalog_example!(support::CatalogExample::new(
    "Edge Label Renderer",
    "Custom labels can be placed at the start, middle, or end of an edge."
)
.initial_data(data)
.show_handles(true)
.handle_positions(Position::Top, Position::Bottom)
.configure_renderer(configure_renderer));

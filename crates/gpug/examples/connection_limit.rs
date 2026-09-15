#![cfg_attr(target_family = "wasm", no_main)]
mod support;
use gpug::{
    Connection, Edge, Graph, GraphData, GraphDataApi, GraphRenderer, HandleKind, Node,
    NodeAppearance, NodeId, NodeShape, WorldPoint, WorldSize,
};
use gpui::{div, px, rgb, App, AppContext, IntoElement, ParentElement, Styled, WindowOptions};

const NODE_ONE: NodeId = NodeId(1);
const NODE_TWO: NodeId = NodeId(2);
const LIMIT_NOTE: NodeId = NodeId(3);

/// A custom handle whose `is_connectable` argument is the maximum number of
/// connections it accepts. This mirrors a numeric `isConnectable` prop.
#[derive(Clone, Copy)]
struct LimitedHandle {
    node: NodeId,
    kind: HandleKind,
    is_connectable: usize,
}

impl LimitedHandle {
    fn accepts(self, connection: &Connection, data: &GraphDataApi) -> bool {
        self.accepts_count(
            connection,
            data.node_connections(self.node, self.kind).len(),
        )
    }

    fn accepts_count(self, connection: &Connection, connection_count: usize) -> bool {
        let proposed = match self.kind {
            HandleKind::Source => connection.source.node == self.node,
            HandleKind::Target => connection.target.node == self.node,
        };
        !proposed || connection_count < self.is_connectable
    }
}

fn invisible_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

fn example_data() -> GraphData {
    let node = |id, position, caption: &str| {
        let mut node = Node::new(id, position)
            .with_size(WorldSize::new(24.0, 7.0))
            .with_type("connection-limit");
        node.metadata.insert("caption".into(), caption.into());
        node
    };

    GraphData::new(
        vec![
            node(NODE_ONE, WorldPoint::new(0.0, 0.0), "Node 1"),
            node(NODE_TWO, WorldPoint::new(0.0, 12.0), "Node 2"),
            node(
                LIMIT_NOTE,
                WorldPoint::new(40.0, 4.0),
                "\u{2190} Only one edge allowed",
            ),
        ],
        vec![Edge::new(NODE_ONE, LIMIT_NOTE)],
    )
}

fn main() {
    support::Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Connection Limit".into()),
                ..Default::default()
            },
            |_, cx| {
                let data_api = GraphDataApi::new();
                let source = LimitedHandle {
                    node: NODE_ONE,
                    kind: HandleKind::Source,
                    is_connectable: 1,
                };
                let validator_data = data_api.clone();
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("connection-limit", invisible_node);
                renderer.register_node_content("connection-limit", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(1.0 * zoom))
                        .border(px(1.0))
                        .border_color(rgb(0xe2e4e7))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px(1.7 * zoom))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });

                cx.new(|cx| {
                    Graph::builder()
                        .data(example_data())
                        .data_api(data_api)
                        .renderer(renderer)
                        .show_handles(true)
                        .fit_on_load()
                        .connection_validator(move |connection| {
                            source.accepts(connection, &validator_data)
                        })
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
    use gpug::HandleKey;

    #[test]
    fn numeric_limit_rejects_a_second_source_connection() {
        let data = example_data();
        let handle = LimitedHandle {
            node: NODE_ONE,
            kind: HandleKind::Source,
            is_connectable: 1,
        };
        let proposed = Connection {
            source: HandleKey {
                node: NODE_ONE,
                id: None,
                kind: HandleKind::Source,
            },
            target: HandleKey {
                node: NODE_TWO,
                id: None,
                kind: HandleKind::Target,
            },
        };

        let connection_count = data
            .edges
            .iter()
            .filter(|edge| edge.source == NODE_ONE)
            .count();
        assert!(!handle.accepts_count(&proposed, connection_count));
    }
}

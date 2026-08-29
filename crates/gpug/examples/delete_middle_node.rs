mod support;

use std::collections::HashSet;

use gpug::{Edge, Graph, GraphData, GraphEvent, Node, WorldPoint, WorldSize};
use gpui::Context;

fn node(id: u64, x: f32, y: f32, caption: &str) -> Node {
    let mut node = Node::new(id, WorldPoint::new(x, y)).with_size(WorldSize::new(14.0, 6.0));
    node.metadata.insert("caption".into(), caption.into());
    node
}

fn initial_data() -> GraphData {
    GraphData::new(
        vec![
            node(1, 0.0, 0.0, "Start here..."),
            node(2, 34.0, 0.0, "...or here!"),
            node(3, 17.0, 13.0, "Delete me."),
            node(4, 25.0, 25.0, "Then me!"),
            node(5, 25.0, 36.0, "End here!"),
        ],
        vec![
            Edge::new_with_id(1_u64, 3_u64, 1_u64),
            Edge::new_with_id(2_u64, 3_u64, 2_u64),
            Edge::new_with_id(3_u64, 4_u64, 3_u64),
            Edge::new_with_id(2_u64, 4_u64, 4_u64),
            Edge::new_with_id(4_u64, 5_u64, 5_u64),
        ],
    )
}

/// Rebuild the working edge list one deleted node at a time. This is the same
/// incomers × outgoers operation used by React Flow's onNodesDelete example,
/// and also handles deleting several adjacent selected nodes in one action.
fn reconnect_deleted_nodes(graph: &mut Graph, event: &GraphEvent, cx: &mut Context<Graph>) {
    let GraphEvent::NodesDeleted {
        deleted,
        connected_edges,
    } = event
    else {
        return;
    };

    let mut working = graph.edges().to_vec();
    working.extend(connected_edges.iter().cloned());
    let mut next_edge_id = working.iter().map(|edge| edge.id.0).max().unwrap_or(0) + 1;
    let deleted_ids = deleted.iter().map(|node| node.id).collect::<HashSet<_>>();

    for deleted_node in deleted {
        let incomers = working
            .iter()
            .filter(|edge| edge.target == deleted_node.id)
            .map(|edge| edge.source)
            .collect::<HashSet<_>>();
        let outgoers = working
            .iter()
            .filter(|edge| edge.source == deleted_node.id)
            .map(|edge| edge.target)
            .collect::<HashSet<_>>();
        working.retain(|edge| edge.source != deleted_node.id && edge.target != deleted_node.id);

        for source in &incomers {
            for target in &outgoers {
                if source != target
                    && !working
                        .iter()
                        .any(|edge| edge.source == *source && edge.target == *target)
                {
                    working.push(Edge::new_with_id(*source, *target, next_edge_id));
                    next_edge_id += 1;
                }
            }
        }
    }

    working
        .retain(|edge| !deleted_ids.contains(&edge.source) && !deleted_ids.contains(&edge.target));
    let existing = graph
        .edges()
        .iter()
        .map(|edge| (edge.source, edge.target))
        .collect::<HashSet<_>>();
    for edge in working {
        if !existing.contains(&(edge.source, edge.target)) {
            let _ = graph.add_edge(edge);
        }
    }
    cx.notify();
}

catalog_example!(support::CatalogExample::new(
    "Delete Middle Node",
    "Delete a node and reconnect every incomer to every outgoer."
)
.initial_data(initial_data)
.show_handles(true)
.on_event(reconnect_deleted_nodes));

mod support;

use gpug::{Edge, Graph, GraphEvent, HandleKind, Node, NodeId, WorldSize};
use gpui::Context;

/// A connection that ends anywhere but a handle leaves `connected` false. That
/// is the whole hook: drop on the pane, get a node there, wired to the handle
/// the drag started from.
fn add_node_on_edge_drop(graph: &mut Graph, event: &GraphEvent, cx: &mut Context<Graph>) {
    let GraphEvent::ConnectEnd {
        from,
        position,
        connected,
        ..
    } = event
    else {
        return;
    };
    if *connected {
        return;
    }

    let node_id = NodeId(
        graph
            .nodes()
            .iter()
            .map(|node| node.id.0)
            .max()
            .unwrap_or(0)
            + 1,
    );
    let edge_id = graph
        .edges()
        .iter()
        .map(|edge| edge.id.0)
        .max()
        .unwrap_or(0)
        + 1;
    let node = Node::new(node_id, *position).with_size(WorldSize::new(11.0, 7.0));
    if graph.add_node(node).is_err() {
        return;
    }
    // The dragged end becomes the far end of the edge: a drag off a source
    // handle points at the new node, a drag off a target handle points back.
    let edge = if from.kind == HandleKind::Source {
        Edge::new(from.node, node_id)
    } else {
        Edge::new(node_id, from.node)
    };
    if graph.add_edge(edge.with_id(edge_id)).is_ok() {
        cx.notify();
    }
}

catalog_example!(support::CatalogExample::new(
    "Add Node On Edge Drop",
    "Drop an unfinished connection on the pane to create and connect a node."
)
.show_handles(true)
.on_event(add_node_on_edge_drop));

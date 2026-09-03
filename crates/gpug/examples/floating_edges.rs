mod support;

use gpug::{Edge, GraphData, Node, WorldPoint, WorldSize};

fn data() -> GraphData {
    let node = |id, x, y| {
        Node::new(id, WorldPoint::new(x, y))
            .with_size(WorldSize::new(15.0, 9.0))
            .with_connectable_body()
    };
    GraphData::new(
        vec![
            node(1_u64, -20.0, -10.0),
            node(2_u64, 20.0, 10.0),
            node(3_u64, -8.0, 20.0),
        ],
        vec![
            Edge::new_with_id(1_u64, 2_u64, 1_u64),
            Edge::new_with_id(3_u64, 2_u64, 2_u64),
        ],
    )
}

catalog_example!(support::CatalogExample::new(
    "Floating Edges",
    "Edges choose facing node sides as their nodes move. Drag from a node body to connect."
)
.initial_data(data));

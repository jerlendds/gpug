#![cfg_attr(target_family = "wasm", no_main)]
mod support;

use gpug::{Edge, GraphData, Node, WorldPoint, WorldSize};

fn data() -> GraphData {
    GraphData::new(
        vec![
            Node::new(1_u64, WorldPoint::new(-18.0, 0.0)).with_size(WorldSize::new(14.0, 8.0)),
            Node::new(2_u64, WorldPoint::new(18.0, 0.0)).with_size(WorldSize::new(14.0, 8.0)),
        ],
        vec![Edge::new_with_id(1_u64, 2_u64, 1_u64)],
    )
}

catalog_example!(support::CatalogExample::new(
    "Simple Floating Edges",
    "The built-in endpoint resolver keeps a straight edge on facing borders."
)
.initial_data(data));

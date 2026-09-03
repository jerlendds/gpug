mod support;

use gpug::{Edge, EdgeMarker, GraphData, Node, WorldPoint, WorldSize};

fn data() -> GraphData {
    let nodes = (0_usize..4)
        .map(|index| {
            Node::new(index + 1, WorldPoint::new(index as f32 * 18.0, 0.0))
                .with_size(WorldSize::new(10.0, 7.0))
        })
        .collect();
    let edges = vec![
        Edge::new_with_id(1_u64, 2_u64, 1_u64).with_marker_end(Some(EdgeMarker::Arrow)),
        Edge::new_with_id(2_u64, 3_u64, 2_u64).with_marker_start(Some(EdgeMarker::ArrowClosed)),
        Edge::new_with_id(3_u64, 4_u64, 3_u64).with_marker_end(None),
    ];
    GraphData::new(nodes, edges)
}

catalog_example!(support::CatalogExample::new(
    "Edge Markers",
    "Open, closed, bidirectional, and marker-free edges."
)
.initial_data(data));

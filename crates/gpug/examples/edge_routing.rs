#![cfg_attr(target_family = "wasm", no_main)]
mod support;

use gpug::{Edge, EdgePathContext, GraphData, GraphRenderer, Node, WorldPoint, WorldSize};

fn orthogonal(context: &EdgePathContext<'_>) -> Vec<WorldPoint> {
    let middle = (context.source.x + context.target.x) * 0.5;
    vec![
        context.source,
        WorldPoint::new(middle, context.source.y),
        WorldPoint::new(middle, context.target.y),
        context.target,
    ]
}

fn renderer(renderer: &mut GraphRenderer) {
    renderer.register_edge_path("orthogonal", orthogonal);
}

fn data() -> GraphData {
    let nodes = vec![
        Node::new(1_u64, WorldPoint::new(-22.0, -10.0)).with_size(WorldSize::new(12.0, 7.0)),
        Node::new(2_u64, WorldPoint::new(22.0, 10.0)).with_size(WorldSize::new(12.0, 7.0)),
        Node::new(3_u64, WorldPoint::new(-22.0, 14.0)).with_size(WorldSize::new(12.0, 7.0)),
        Node::new(4_u64, WorldPoint::new(22.0, -14.0)).with_size(WorldSize::new(12.0, 7.0)),
    ];
    let mut routed = Edge::new_with_id(1_u64, 2_u64, 1_u64);
    routed.edge_type = "orthogonal".into();
    routed.label = Some("orthogonal".into());
    let mut curved = Edge::new_with_id(3_u64, 4_u64, 2_u64);
    curved.edge_type = "bezier".into();
    curved.label = Some("bezier".into());
    GraphData::new(nodes, vec![routed, curved])
}

catalog_example!(support::CatalogExample::new(
    "Edge Routing",
    "A public custom path renderer supplies world-space route points."
)
.initial_data(data)
.configure_renderer(renderer));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthogonal_route_includes_bends_and_endpoints() {
        let edge = Edge::new_with_id(1_u64, 2_u64, 1_u64);
        let route = orthogonal(&EdgePathContext {
            edge: &edge,
            source: WorldPoint::new(0.0, 0.0),
            target: WorldPoint::new(10.0, 8.0),
        });
        assert_eq!(route.len(), 4);
        assert_eq!(route[0], WorldPoint::new(0.0, 0.0));
        assert_eq!(route[3], WorldPoint::new(10.0, 8.0));
    }
}

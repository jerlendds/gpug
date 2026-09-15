use gpug::analysis::{spatial::*, *};
use gpug::gpug_bridge::*;
use std::{collections::HashSet, sync::Arc};

#[test]
fn focused_overlapping_source_keeps_custom_renderer_and_exact_connections() {
    let nodes = (0..257u64)
        .map(|id| {
            let mut node = Node::new(
                id,
                WorldRect {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                },
            );
            node.renderer = "flow-form".into();
            node
        })
        .collect();
    let edges = (0..257u64)
        .map(|id| Edge::new(id, id, (id + 1) % 257))
        .collect();
    let index = Arc::new(SpatialIndex::new(Arc::new(
        GraphSnapshot::new(nodes, edges).unwrap(),
    )));
    let plan = Arc::new(
        LodWorkspace::new()
            .prepare_focused(
                index,
                WorldRect {
                    x: -1.,
                    y: -1.,
                    width: 12.,
                    height: 12.,
                },
                LodOptions {
                    zoom: 60.,
                    max_items: 3,
                    max_nodes: 1,
                    max_branches: 1,
                    ..Default::default()
                },
                Some(NodeId(128)),
            )
            .unwrap(),
    );
    let connections = plan
        .connections(ConnectionBudget::default(), || false)
        .unwrap();
    let view = to_lod_graph_data(connections, "aggregate").unwrap();
    let focused = view
        .data
        .nodes
        .iter()
        .find(|node| node.id.0 == 128)
        .unwrap();
    assert_eq!(focused.node_type, "flow-form");
    assert_eq!(view.data.nodes.last().unwrap().id, focused.id);
    let owner = view.node_item(focused.id).unwrap();
    assert_eq!(
        view.connections()
            .plan()
            .members(owner)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![NodeId(128)]
    );
    let mut edge_ids = HashSet::new();
    for group in 0..view.connections().groups().len() {
        for id in view.connections().members(group).unwrap() {
            assert!(edge_ids.insert(id));
        }
    }
    assert_eq!(edge_ids, (0..257).map(EdgeId).collect());
}

fn summary(max_items: usize, viewport: WorldRect) -> PlanConnections {
    let nodes: Vec<_> = (0..33u64)
        .map(|index| {
            let mut node = Node::new(
                if index == 32 { u64::MAX } else { index },
                WorldRect {
                    x: index as f64 * 10.,
                    y: 0.,
                    width: 5.,
                    height: 5.,
                },
            );
            node.renderer = "flow-form".into();
            node.label = format!("Node {index}");
            node
        })
        .collect();
    let mut edges: Vec<_> = nodes
        .windows(2)
        .enumerate()
        .map(|(id, pair)| Edge::new(100 + id as u64, pair[0].id, pair[1].id))
        .collect();
    edges.push(Edge::new(200, 0, 0));
    let index = Arc::new(SpatialIndex::new(Arc::new(
        GraphSnapshot::new(nodes, edges).unwrap(),
    )));
    let plan = Arc::new(
        LodWorkspace::new()
            .prepare(
                index,
                viewport,
                LodOptions {
                    zoom: 60.,
                    max_items,
                    ..Default::default()
                },
            )
            .unwrap(),
    );
    plan.connections(ConnectionBudget::default(), || false)
        .unwrap()
}

#[test]
fn mixed_scene_preserves_detailed_renderers_and_aggregate_provenance_without_id_collisions() {
    let view = to_lod_graph_data(
        summary(
            20,
            WorldRect {
                x: 0.,
                y: 0.,
                width: 330.,
                height: 5.,
            },
        ),
        "aggregate",
    )
    .unwrap();
    let source = view.connections().plan().index().snapshot();
    let mut detailed = 0;
    let mut aggregates = 0;
    let mut ids = HashSet::new();
    for node in &view.data.nodes {
        assert!(ids.insert(node.id));
        let item = view.node_item(node.id).unwrap();
        let plan_item = &view.connections().plan().items()[item];
        assert!(!node.draggable && !node.deletable && !node.connectable);
        match plan_item.kind {
            LodKind::Node(id) => {
                detailed += 1;
                assert_eq!(node.id.0, id.0);
                assert_eq!(node.node_type, "flow-form");
                assert_eq!(
                    node.metadata[LABEL_KEY],
                    source.nodes()[source.node_index(id).unwrap() as usize].label
                );
            }
            LodKind::Aggregate => {
                aggregates += 1;
                assert!(source.node_index(NodeId(node.id.0)).is_none());
                assert_eq!(node.node_type, "aggregate");
                assert_eq!(
                    node.metadata[AGGREGATE_COUNT_KEY],
                    plan_item.represented_nodes.to_string()
                );
            }
        }
    }
    assert!(detailed > 0 && aggregates > 0);
    view.data.compile_edges().unwrap();
    let rendered: usize = view
        .data
        .edges
        .iter()
        .map(|edge| {
            let group = &view.connections().groups()[view.edge_group(edge.id).unwrap()];
            assert_eq!(edge.label, Some(format!("{} edges", group.edge_count)));
            assert_eq!(edge.marker_end, Some(gpug::EdgeMarker::ArrowClosed));
            assert!(!edge.deletable && !edge.reconnectable);
            group.edge_count
        })
        .sum();
    let context: usize = view
        .context_groups()
        .iter()
        .map(|index| view.connections().groups()[*index].edge_count)
        .sum();
    assert_eq!(
        rendered + context + view.connections().hidden_edges,
        source.edges().len()
    );
    assert!(view.edge_group(gpug::EdgeId(u64::MAX)).is_none());
}

#[test]
fn outside_and_internal_connections_have_explicit_context_even_with_no_drawable_edges() {
    let outside = to_lod_graph_data(
        summary(
            100,
            WorldRect {
                x: 1000.,
                width: 10.,
                height: 10.,
                ..Default::default()
            },
        ),
        "aggregate",
    )
    .unwrap();
    assert!(outside.data.nodes.is_empty() && outside.data.edges.is_empty());
    assert_eq!(outside.context_groups().len(), 1);
    assert_eq!(
        outside
            .connections()
            .members(outside.context_groups()[0])
            .unwrap()
            .count(),
        33
    );
    let coarse = to_lod_graph_data(
        summary(
            1,
            WorldRect {
                width: 330.,
                height: 10.,
                ..Default::default()
            },
        ),
        "aggregate",
    )
    .unwrap();
    assert_eq!(coarse.data.nodes.len(), 1);
    assert!(coarse.data.edges.is_empty());
    assert_eq!(coarse.data.nodes[0].metadata[INTERNAL_EDGES_KEY], "33");
    assert_eq!(coarse.context_groups().len(), 1);
}

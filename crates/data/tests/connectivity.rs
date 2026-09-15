use gpug_data::{spatial::*, *};
use std::{collections::HashMap, sync::Arc};

#[test]
fn small_and_large_owner_domains_preserve_first_seen_groups_and_weight_order() {
    // Identical connected nodes, with enough isolated represented nodes in the
    // second case to rule out quadratic pair storage. Parallel handles exercise
    // accumulation without violating connection uniqueness.
    for count in [3u64, 1103] {
        let nodes = (0..count)
            .map(|id| {
                Node::new(
                    id,
                    WorldRect {
                        x: id as f64 * 10.,
                        y: 0.,
                        width: 1.,
                        height: 1.,
                    },
                )
            })
            .collect();
        let edges = (0..1000u64)
            .map(|id| {
                let source = if id % 2 == 0 { 2 } else { 0 };
                let mut edge = Edge::new(id, source, 1);
                edge.source_handle = Some(id.to_string());
                edge.weight = if id < 2 { 1e16 } else { 1. };
                edge
            })
            .collect();
        let graph = Arc::new(GraphSnapshot::new(nodes, edges).unwrap());
        let mut expected = [0f64; 2];
        for edge in graph.edges() {
            expected[usize::from(edge.source == NodeId(0))] += edge.weight;
        }
        let index = Arc::new(SpatialIndex::new(Arc::clone(&graph)));
        let plan = Arc::new(
            LodWorkspace::new()
                .prepare(
                    index,
                    WorldRect {
                        x: -1.,
                        y: -1.,
                        width: count as f64 * 10. + 2.,
                        height: 3.,
                    },
                    LodOptions {
                        zoom: 60.,
                        max_items: 2048,
                        ..Default::default()
                    },
                )
                .unwrap(),
        );
        assert_eq!(plan.items().len(), count as usize);
        let summary = plan
            .connections(ConnectionBudget::default(), || false)
            .unwrap();
        assert_eq!(summary.groups().len(), 2);
        for (group_index, source) in [NodeId(2), NodeId(0)].into_iter().enumerate() {
            let group = &summary.groups()[group_index];
            assert_eq!(
                group.source,
                PlanEndpoint::Item(summary.node_item(source).unwrap())
            );
            assert_eq!(
                group.target,
                PlanEndpoint::Item(summary.node_item(NodeId(1)).unwrap())
            );
            assert_eq!(group.edge_count, 500);
            assert_eq!(group.weight_sum.to_bits(), expected[group_index].to_bits());
            assert_eq!(
                summary.members(group_index).unwrap().collect::<Vec<_>>(),
                graph
                    .edges()
                    .iter()
                    .filter(|edge| edge.source == source)
                    .map(|edge| edge.id)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn exhaustive_graphs_conserve_direction_counts_weights_and_exact_edge_membership() {
    for mask in 0..512 {
        let nodes = (0..3u64)
            .map(|id| {
                Node::new(
                    u64::MAX - id,
                    WorldRect {
                        x: id as f64 * 10.,
                        y: 0.,
                        width: 1.,
                        height: 1.,
                    },
                )
            })
            .collect();
        let mut edges = Vec::new();
        for source in 0..3u64 {
            for target in 0..3u64 {
                let bit = source * 3 + target;
                if mask & (1 << bit) != 0 {
                    let mut edge = Edge::new(bit + 100, u64::MAX - source, u64::MAX - target);
                    edge.weight = (bit + 1) as f64;
                    edges.push(edge);
                }
            }
        }
        let graph = Arc::new(GraphSnapshot::new(nodes, edges).unwrap());
        let index = Arc::new(SpatialIndex::new(Arc::clone(&graph)));
        let mut workspace = LodWorkspace::new();
        for viewport in [
            WorldRect {
                x: -1.,
                y: -1.,
                width: 30.,
                height: 3.,
            },
            WorldRect {
                x: 10.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
            WorldRect {
                x: 100.,
                y: 0.,
                width: 1.,
                height: 1.,
            },
        ] {
            for max_items in [1, 3] {
                let plan = Arc::new(
                    workspace
                        .prepare(
                            Arc::clone(&index),
                            viewport,
                            LodOptions {
                                zoom: 60.,
                                max_items,
                                ..Default::default()
                            },
                        )
                        .unwrap(),
                );
                let summary = plan
                    .connections(ConnectionBudget::default(), || false)
                    .unwrap();
                let owners: HashMap<_, _> = plan
                    .items()
                    .iter()
                    .enumerate()
                    .flat_map(|(item, _)| {
                        plan.members(item)
                            .unwrap()
                            .map(move |id| (id, PlanEndpoint::Item(item)))
                    })
                    .collect();
                assert_eq!(summary.hidden_edges, 0);
                for node in graph.nodes() {
                    let expected = match owners.get(&node.id) {
                        Some(PlanEndpoint::Item(item)) => Some(*item),
                        _ => None,
                    };
                    assert_eq!(summary.node_item(node.id), expected);
                }
                assert_eq!(summary.node_item(NodeId(42)), None);
                assert_eq!(
                    summary
                        .groups()
                        .iter()
                        .map(|group| group.edge_count)
                        .sum::<usize>(),
                    graph.edges().len()
                );
                for (index, group) in summary.groups().iter().enumerate() {
                    let expected: Vec<_> = graph
                        .edges()
                        .iter()
                        .filter(|edge| {
                            owners
                                .get(&edge.source)
                                .copied()
                                .unwrap_or(PlanEndpoint::Outside)
                                == group.source
                                && owners
                                    .get(&edge.target)
                                    .copied()
                                    .unwrap_or(PlanEndpoint::Outside)
                                    == group.target
                        })
                        .collect();
                    assert_eq!(group.edge_count, expected.len());
                    assert_eq!(
                        group.weight_sum,
                        expected.iter().map(|edge| edge.weight).sum::<f64>()
                    );
                    assert_eq!(
                        summary.members(index).unwrap().collect::<Vec<_>>(),
                        expected.iter().map(|edge| edge.id).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn outside_crossing_edges_parallel_handles_and_hidden_incidence_stay_explicit() {
    let mut hidden = Node::new(3, WorldRect::default());
    hidden.hidden = true;
    let mut parallel = Edge::new(2, 1, 2);
    parallel.source_handle = Some("other".into());
    let graph = Arc::new(
        GraphSnapshot::new(
            vec![
                Node::new(
                    1,
                    WorldRect {
                        x: -100.,
                        ..Default::default()
                    },
                ),
                Node::new(
                    2,
                    WorldRect {
                        x: 100.,
                        ..Default::default()
                    },
                ),
                hidden,
            ],
            vec![
                Edge::new(1, 1, 2),
                parallel,
                Edge::new(3, 2, 1),
                Edge::new(4, 3, 1),
            ],
        )
        .unwrap(),
    );
    let index = Arc::new(SpatialIndex::new(graph));
    let mut workspace = LodWorkspace::new();
    let plan = Arc::new(
        workspace
            .prepare(
                Arc::clone(&index),
                WorldRect::default(),
                LodOptions {
                    zoom: 60.,
                    ..Default::default()
                },
            )
            .unwrap(),
    );
    assert!(plan.items().is_empty());
    let summary = plan
        .connections(ConnectionBudget::default(), || false)
        .unwrap();
    assert_eq!(summary.groups().len(), 1);
    assert_eq!(summary.groups()[0].source, PlanEndpoint::Outside);
    assert_eq!(summary.groups()[0].target, PlanEndpoint::Outside);
    assert_eq!(summary.groups()[0].edge_count, 3);
    assert_eq!(summary.hidden_edges, 1);
    assert_eq!(
        summary.members(0).unwrap().collect::<Vec<_>>(),
        vec![EdgeId(1), EdgeId(2), EdgeId(3)]
    );
    // Reusing the planner cannot change an already-published summary or its source.
    workspace
        .prepare(
            index,
            WorldRect {
                x: -100.,
                width: 200.,
                ..Default::default()
            },
            LodOptions::default(),
        )
        .unwrap();
    assert!(summary.plan().items().is_empty());
    assert_eq!(summary.members(0).unwrap().count(), 3);
}

#[test]
fn limits_cancellation_and_overflow_never_publish_partial_summaries() {
    let mut a = Edge::new(1, 1, 1);
    a.weight = f64::MAX;
    let mut b = Edge::new(2, 1, 1);
    b.weight = f64::MAX;
    b.source_handle = Some("parallel".into());
    let index = Arc::new(SpatialIndex::new(Arc::new(
        GraphSnapshot::new(vec![Node::new(1, WorldRect::default())], vec![a, b]).unwrap(),
    )));
    let plan = Arc::new(
        LodWorkspace::new()
            .prepare(index, WorldRect::default(), LodOptions::default())
            .unwrap(),
    );
    for (budget, expected) in [
        (
            ConnectionBudget {
                max_nodes: 0,
                ..Default::default()
            },
            ConnectionPlanError::NodeLimit,
        ),
        (
            ConnectionBudget {
                max_edges: 0,
                ..Default::default()
            },
            ConnectionPlanError::EdgeLimit,
        ),
        (
            ConnectionBudget {
                max_groups: 0,
                ..Default::default()
            },
            ConnectionPlanError::GroupLimit,
        ),
        (
            ConnectionBudget::default(),
            ConnectionPlanError::WeightOverflow,
        ),
    ] {
        assert_eq!(plan.connections(budget, || false).unwrap_err(), expected);
    }
    assert_eq!(
        plan.connections(ConnectionBudget::default(), || true)
            .unwrap_err(),
        ConnectionPlanError::Cancelled
    );
    let mut checks = 0;
    assert_eq!(
        plan.connections(ConnectionBudget::default(), || {
            checks += 1;
            checks == 3
        })
        .unwrap_err(),
        ConnectionPlanError::Cancelled
    );
}

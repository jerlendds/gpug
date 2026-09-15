use gpug_data::{spatial::*, *};
use std::{collections::HashSet, sync::Arc};

fn fixture(overlap: bool) -> Arc<GraphSnapshot> {
    Arc::new(
        GraphSnapshot::new(
            (0..257u64)
                .map(|id| {
                    let mut node = Node::new(
                        u64::MAX - id,
                        WorldRect {
                            x: if overlap {
                                0.
                            } else {
                                (id % 17) as f64 * 10. - 80.
                            },
                            y: if overlap {
                                0.
                            } else {
                                (id / 17) as f64 * 10. - 80.
                            },
                            width: 3.,
                            height: 7.,
                        },
                    );
                    node.hidden = id % 11 == 0;
                    node
                })
                .collect(),
            vec![],
        )
        .unwrap(),
    )
}
fn visible(node: &Node, viewport: WorldRect) -> bool {
    let b = node.bounds;
    !node.hidden
        && b.x <= viewport.x + viewport.width
        && viewport.x <= b.x + b.width
        && b.y <= viewport.y + viewport.height
        && viewport.y <= b.y + b.height
}

#[test]
fn focus_extracts_exact_membership_without_exceeding_hard_budgets() {
    let viewport = WorldRect {
        x: -100.,
        y: -100.,
        width: 300.,
        height: 300.,
    };
    let mut workspace = LodWorkspace::new();
    for overlap in [false, true] {
        let graph = fixture(overlap);
        let index = Arc::new(SpatialIndex::new(Arc::clone(&graph)));
        let expected = graph
            .nodes()
            .iter()
            .filter(|node| !node.hidden)
            .map(|node| node.id)
            .collect::<HashSet<_>>();
        for node in graph.nodes() {
            for max_items in [1, 2, 3, 16] {
                for max_nodes in [0, 1, 8] {
                    let plan = workspace
                        .prepare_focused(
                            Arc::clone(&index),
                            viewport,
                            LodOptions {
                                zoom: 60.,
                                max_items,
                                max_nodes,
                                max_branches: 1,
                                ..Default::default()
                            },
                            Some(node.id),
                        )
                        .unwrap();
                    assert!(plan.items().len() <= max_items);
                    assert!(plan.examined_nodes <= max_nodes);
                    assert!(plan.visited_branches <= 1);
                    let mut covered = HashSet::new();
                    for (slot, item) in plan.items().iter().enumerate() {
                        let members = plan.members(slot).unwrap().collect::<Vec<_>>();
                        assert_eq!(members.len(), item.represented_nodes);
                        for member in members {
                            assert!(covered.insert(member));
                        }
                    }
                    assert_eq!(covered, expected);
                    if !node.hidden && max_items >= 3 && max_nodes >= 1 {
                        assert_eq!(plan.items().last().unwrap().kind, LodKind::Node(node.id));
                        let item = plan
                            .items()
                            .iter()
                            .find(|item| item.kind == LodKind::Node(node.id))
                            .expect("focused custom-content node");
                        assert_eq!(item.bounds, node.bounds);
                    }
                }
            }
        }
        for focus in [Some(NodeId(42)), None] {
            let plan = workspace
                .prepare_focused(Arc::clone(&index), viewport, LodOptions::default(), focus)
                .unwrap();
            let ordinary = workspace
                .prepare(Arc::clone(&index), viewport, LodOptions::default())
                .unwrap();
            assert_eq!(plan.items().len(), ordinary.items().len());
            for slot in 0..plan.items().len() {
                assert_eq!(
                    plan.members(slot).unwrap().collect::<Vec<_>>(),
                    ordinary.members(slot).unwrap().collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn bounded_plans_cover_every_visible_node_with_disjoint_exact_provenance() {
    let mut workspace = LodWorkspace::new();
    for overlap in [false, true] {
        let snapshot = fixture(overlap);
        let index = SpatialIndex::new(Arc::clone(&snapshot));
        for x in [-100., -10., 0., 30., 100.] {
            let viewport = WorldRect {
                x,
                y: -20.,
                width: 43.,
                height: 51.,
            };
            for max_items in [1, 2, 7, 16, 64, 512] {
                for max_branches in [1, 2, 3, 7, 64] {
                    for max_nodes in [0, 15, 64, 512] {
                        let options = LodOptions {
                            max_items,
                            max_branches,
                            max_nodes,
                            zoom: 60.,
                            ..Default::default()
                        };
                        let plan = workspace.plan(&index, viewport, options).unwrap();
                        assert!(plan.items().len() <= max_items);
                        assert!(plan.visited_branches <= max_branches);
                        assert!(plan.examined_nodes <= max_nodes);
                        let mut covered = HashSet::new();
                        for (i, item) in plan.items().iter().enumerate() {
                            assert!(item.bounds.is_valid());
                            assert!(item.bounds.intersects(viewport));
                            let members: Vec<_> = plan.members(i).unwrap().collect();
                            assert_eq!(members.len(), item.represented_nodes);
                            for id in members {
                                assert!(
                                    covered.insert(id),
                                    "aggregate membership must be disjoint"
                                );
                                let node =
                                    snapshot.nodes().iter().find(|node| node.id == id).unwrap();
                                assert!(!node.hidden);
                                if let LodKind::Node(detailed) = item.kind {
                                    assert_eq!(detailed, id);
                                    assert_eq!(item.bounds, node.bounds);
                                }
                            }
                        }
                        assert!(plan.members(plan.items().len()).is_none());
                        for node in snapshot
                            .nodes()
                            .iter()
                            .filter(|node| visible(node, viewport))
                        {
                            assert!(
                                covered.contains(&node.id),
                                "budget must not cause a visibility hole"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn close_zoom_refines_small_groups_and_hard_budgets_keep_fallbacks() {
    let index = SpatialIndex::new(fixture(true));
    let viewport = WorldRect {
        x: -1.,
        y: -1.,
        width: 10.,
        height: 10.,
    };
    let mut workspace = LodWorkspace::new();
    let options = LodOptions {
        zoom: 1.,
        max_zoom: 2.,
        max_items: 512,
        max_nodes: 512,
        ..Default::default()
    };
    let plan = workspace.plan(&index, viewport, options).unwrap();
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].kind, LodKind::Aggregate);
    assert_eq!(plan.items()[0].represented_nodes, index.indexed_nodes());
    assert_eq!(plan.examined_nodes, 0);
    let plan = workspace
        .plan(
            &index,
            viewport,
            LodOptions {
                zoom: 1.8,
                ..options
            },
        )
        .unwrap();
    assert_eq!(plan.items().len(), index.indexed_nodes());
    assert!(plan
        .items()
        .iter()
        .all(|item| matches!(item.kind, LodKind::Node(_))));
    let plan = workspace
        .plan(
            &index,
            viewport,
            LodOptions {
                zoom: 2.,
                max_items: 1,
                ..options
            },
        )
        .unwrap();
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.budget_aggregates, 1);
    assert_eq!(plan.items()[0].represented_nodes, index.indexed_nodes());
}

#[test]
fn extremes_empty_input_and_invalid_options_are_handled() {
    let index = SpatialIndex::new(Arc::new(
        GraphSnapshot::new(
            vec![
                Node::new(
                    1,
                    WorldRect {
                        x: -f64::MAX,
                        ..Default::default()
                    },
                ),
                Node::new(
                    2,
                    WorldRect {
                        x: f64::MAX,
                        ..Default::default()
                    },
                ),
            ],
            vec![],
        )
        .unwrap(),
    ));
    let mut workspace = LodWorkspace::new();
    for viewport in [
        WorldRect::default(),
        WorldRect {
            x: -f64::MAX,
            width: f64::MAX,
            ..Default::default()
        },
    ] {
        let plan = workspace
            .plan(
                &index,
                viewport,
                LodOptions {
                    max_nodes: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(plan.items()[0].bounds.is_valid());
        assert_eq!(plan.members(0).unwrap().count(), 2);
    }
    for options in [
        LodOptions {
            zoom: f64::NAN,
            ..Default::default()
        },
        LodOptions {
            max_zoom: 0.,
            ..Default::default()
        },
        LodOptions {
            aggregate_below_pixels: -1.,
            ..Default::default()
        },
        LodOptions {
            full_detail_zoom_ratio: 1.1,
            ..Default::default()
        },
        LodOptions {
            max_items: 0,
            ..Default::default()
        },
        LodOptions {
            max_branches: 0,
            ..Default::default()
        },
    ] {
        assert!(workspace
            .plan(&index, WorldRect::default(), options)
            .is_err());
    }
    assert!(workspace
        .plan(
            &index,
            WorldRect {
                x: f64::NAN,
                ..Default::default()
            },
            LodOptions::default()
        )
        .is_err());
    let empty = SpatialIndex::new(Arc::new(GraphSnapshot::new(vec![], vec![]).unwrap()));
    assert!(workspace
        .plan(&empty, WorldRect::default(), LodOptions::default())
        .unwrap()
        .items()
        .is_empty());
}

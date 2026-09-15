use gpug_data::{spatial::*, *};
use std::sync::Arc;

fn unlimited() -> SpatialBudget {
    SpatialBudget {
        max_branches: usize::MAX,
        max_nodes: usize::MAX,
        max_results: usize::MAX,
    }
}
fn check(index: &SpatialIndex, workspace: &mut SpatialWorkspace, viewport: WorldRect) {
    let result = workspace.query(index, viewport, unlimited()).unwrap();
    assert!(!result.truncated);
    let mut actual = result.nodes.to_vec();
    actual.sort_unstable();
    let mut expected: Vec<_> = index
        .snapshot()
        .nodes()
        .iter()
        .filter(|node| {
            let b = node.bounds;
            !node.hidden
                && b.x <= viewport.x + viewport.width
                && viewport.x <= b.x + b.width
                && b.y <= viewport.y + viewport.height
                && viewport.y <= b.y + b.height
        })
        .map(|node| node.id)
        .collect();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn many_viewports_match_independent_linear_rectangle_scan() {
    let nodes = (0..4096u64)
        .map(|id| {
            let mut node = Node::new(
                u64::MAX - id,
                WorldRect {
                    x: (id % 64) as f64 * 11. - 350.,
                    y: (id / 64) as f64 * 7. - 200.,
                    width: (id % 13) as f64,
                    height: (id % 17) as f64,
                },
            );
            node.hidden = id % 7 == 0;
            node
        })
        .collect();
    let index = SpatialIndex::new(Arc::new(GraphSnapshot::new(nodes, vec![]).unwrap()));
    let mut workspace = SpatialWorkspace::new();
    for x in -20..20 {
        for y in -10..10 {
            check(
                &index,
                &mut workspace,
                WorldRect {
                    x: f64::from(x * 25),
                    y: f64::from(y * 25),
                    width: 53.,
                    height: 31.,
                },
            );
        }
    }
    let result = workspace
        .query(
            &index,
            WorldRect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            unlimited(),
        )
        .unwrap();
    assert!(result.examined_nodes < index.indexed_nodes() / 10);
}

#[test]
fn touching_zero_area_hidden_and_extreme_unions_are_conservative() {
    let mut nodes = vec![
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
    ];
    nodes.extend((3..100).map(|id| {
        Node::new(
            id,
            WorldRect {
                x: 0.,
                y: 0.,
                width: 2.,
                height: 2.,
            },
        )
    }));
    nodes[2].hidden = true;
    let snapshot = Arc::new(GraphSnapshot::new(nodes, vec![]).unwrap());
    let index = SpatialIndex::new(Arc::clone(&snapshot));
    assert!(Arc::ptr_eq(index.snapshot(), &snapshot));
    let mut workspace = SpatialWorkspace::new();
    for x in [-f64::MAX, f64::MAX, 0., 2., 2.1] {
        check(
            &index,
            &mut workspace,
            WorldRect {
                x,
                ..Default::default()
            },
        );
    }
    let hidden = Arc::new(
        GraphSnapshot::new(
            vec![{
                let mut node = Node::new(1, WorldRect::default());
                node.hidden = true;
                node
            }],
            vec![],
        )
        .unwrap(),
    );
    assert_eq!(SpatialIndex::new(hidden).indexed_nodes(), 0);
    check(
        &SpatialIndex::new(Arc::new(GraphSnapshot::new(vec![], vec![]).unwrap())),
        &mut workspace,
        WorldRect::default(),
    );
}

#[test]
fn all_work_limits_report_incomplete_and_reuse_recovers() {
    let graph = Arc::new(
        GraphSnapshot::new(
            (0..100)
                .map(|id| Node::new(id, WorldRect::default()))
                .collect(),
            vec![],
        )
        .unwrap(),
    );
    let index = SpatialIndex::new(graph);
    let mut workspace = SpatialWorkspace::new();
    for budget in [
        SpatialBudget {
            max_branches: 0,
            ..unlimited()
        },
        SpatialBudget {
            max_branches: 2,
            ..unlimited()
        },
        SpatialBudget {
            max_nodes: 0,
            ..unlimited()
        },
        SpatialBudget {
            max_nodes: 17,
            ..unlimited()
        },
        SpatialBudget {
            max_results: 0,
            ..unlimited()
        },
        SpatialBudget {
            max_results: 17,
            ..unlimited()
        },
    ] {
        let result = workspace
            .query(&index, WorldRect::default(), budget)
            .unwrap();
        assert!(result.truncated);
        assert!(result.visited_branches <= budget.max_branches);
        assert!(result.examined_nodes <= budget.max_nodes);
        assert!(result.nodes.len() <= budget.max_results);
    }
    let result = workspace
        .query(
            &index,
            WorldRect::default(),
            SpatialBudget {
                max_results: 100,
                ..unlimited()
            },
        )
        .unwrap();
    assert!(!result.truncated);
    assert_eq!(result.nodes.len(), 100);
    for viewport in [
        WorldRect {
            x: f64::NAN,
            ..Default::default()
        },
        WorldRect {
            width: -1.,
            ..Default::default()
        },
        WorldRect {
            x: f64::MAX,
            width: f64::MAX,
            ..Default::default()
        },
    ] {
        assert!(workspace.query(&index, viewport, unlimited()).is_err());
    }
    check(&index, &mut workspace, WorldRect::default());
}

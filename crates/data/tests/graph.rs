use gpug_data::*;

fn node(id: u64) -> Node {
    Node::new(id, WorldRect::default())
}
fn graph() -> GraphSnapshot {
    GraphSnapshot::new(
        vec![node(10), node(20), node(30), node(40)],
        vec![
            Edge::new(7, 10, 20),
            Edge::new(8, 20, 30),
            Edge::new(9, 30, 10),
        ],
    )
    .unwrap()
}

#[test]
fn invalid_snapshots_and_transactions_are_atomic() {
    assert!(matches!(
        GraphSnapshot::new(vec![node(1), node(1)], vec![]),
        Err(GraphError::DuplicateNode(NodeId(1)))
    ));
    assert!(matches!(
        GraphSnapshot::new(vec![node(1)], vec![Edge::new(1, 1, 2)]),
        Err(GraphError::UnknownNode(NodeId(2)))
    ));
    let g = graph();
    assert!(g
        .transact([
            Mutation::RemoveNode(NodeId(20)),
            Mutation::AddEdge(Edge::new(11, 10, 20))
        ])
        .is_err());
    assert_eq!(g.nodes().len(), 4);
    assert_eq!(g.edges().len(), 3);
    let next = g.transact([Mutation::RemoveNode(NodeId(20))]).unwrap();
    assert_eq!(
        next.edges().iter().map(|e| e.id.0).collect::<Vec<_>>(),
        vec![9]
    );
    assert_eq!(next.endpoint_columns(), (&[1][..], &[0][..]));
}

#[test]
fn geometry_and_connections_are_checked() {
    for bounds in [
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
        assert!(GraphSnapshot::new(vec![Node::new(1, bounds)], vec![]).is_err());
    }
    let first = Edge::new(1, 1, 1);
    let mut second = Edge::new(2, 1, 1);
    assert!(GraphSnapshot::new(vec![node(1)], vec![first.clone(), second.clone()]).is_err());
    second.target_handle = Some("other".into());
    let g = GraphSnapshot::new(vec![node(1)], vec![first, second]).unwrap();
    assert_eq!(g.degree(NodeId(1)).unwrap(), (2, 2));
}

#[test]
fn direction_hops_and_work_limits_are_explicit() {
    let g = graph();
    let mut workspace = SearchWorkspace::new();
    let budget = SearchBudget {
        max_hops: 2,
        max_nodes: 100,
        max_edges: 100,
    };
    let result = workspace
        .neighborhood(&g, NodeId(10), Direction::Outgoing, budget)
        .unwrap();
    assert_eq!(result.nodes, &[NodeId(10), NodeId(20), NodeId(30)]);
    assert_eq!(result.depths, &[0, 1, 2]);
    assert!(!result.truncated);
    let result = workspace
        .neighborhood(&g, NodeId(10), Direction::Incoming, budget)
        .unwrap();
    assert_eq!(result.nodes, &[NodeId(10), NodeId(30), NodeId(20)]);
    let result = workspace
        .neighborhood(
            &g,
            NodeId(10),
            Direction::Both,
            SearchBudget {
                max_edges: 0,
                ..budget
            },
        )
        .unwrap();
    assert!(result.truncated);
    assert_eq!(result.examined_edges, 0);
    let result = workspace
        .neighborhood(
            &g,
            NodeId(10),
            Direction::Both,
            SearchBudget {
                max_nodes: 0,
                ..budget
            },
        )
        .unwrap();
    assert!(result.truncated && result.nodes.is_empty());
    assert_eq!(g.weak_components(), vec![0, 0, 0, 1]);
}

#[test]
fn induced_subgraph_preserves_identity_and_order() {
    let g = graph()
        .induced_subgraph(&[NodeId(30), NodeId(10), NodeId(30)])
        .unwrap();
    assert_eq!(
        g.nodes().iter().map(|n| n.id.0).collect::<Vec<_>>(),
        vec![10, 30]
    );
    assert_eq!(g.edges()[0].id, EdgeId(9));
}

// Independent edge-list oracle, deliberately avoiding the optimized CSR indexes.
#[test]
fn exhaustive_small_graphs_match_edge_list_reachability() {
    let mut workspace = SearchWorkspace::new();
    for mask in 0u32..512 {
        let edges: Vec<_> = (0..9)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| Edge::new(i, i / 3, i % 3))
            .collect();
        let g = GraphSnapshot::new((0..3).map(node).collect(), edges).unwrap();
        for seed in 0..3 {
            for direction in [Direction::Incoming, Direction::Outgoing, Direction::Both] {
                for hops in 0..4 {
                    let mut distances = [usize::MAX; 3];
                    distances[seed as usize] = 0;
                    for depth in 0..hops {
                        let old = distances;
                        for edge in g.edges() {
                            if direction != Direction::Incoming
                                && old[edge.source.0 as usize] == depth
                            {
                                distances[edge.target.0 as usize] =
                                    distances[edge.target.0 as usize].min(depth + 1);
                            }
                            if direction != Direction::Outgoing
                                && old[edge.target.0 as usize] == depth
                            {
                                distances[edge.source.0 as usize] =
                                    distances[edge.source.0 as usize].min(depth + 1);
                            }
                        }
                    }
                    let result = workspace
                        .neighborhood(
                            &g,
                            NodeId(seed),
                            direction,
                            SearchBudget {
                                max_hops: hops,
                                max_nodes: 3,
                                max_edges: 18,
                            },
                        )
                        .unwrap();
                    let mut actual = [usize::MAX; 3];
                    for (id, depth) in result.nodes.iter().zip(result.depths) {
                        actual[id.0 as usize] = *depth;
                    }
                    assert_eq!(
                        actual, distances,
                        "mask={mask}, seed={seed}, direction={direction:?}, hops={hops}"
                    );
                    assert!(!result.truncated);
                }
            }
        }
    }
}

#[test]
fn cancellation_bounds_hub_traversal_and_workspace_can_be_reused() {
    let graph = GraphSnapshot::new(
        (0..1025).map(node).collect(),
        (1..1025).map(|id| Edge::new(id, 0, id)).collect(),
    )
    .unwrap();
    let mut workspace = SearchWorkspace::new();
    let result = workspace
        .neighborhood_cancellable(
            &graph,
            NodeId(0),
            Direction::Outgoing,
            SearchBudget::default(),
            || true,
        )
        .unwrap();
    assert!(result.truncated);
    assert!(result.nodes.is_empty());
    assert_eq!(result.examined_edges, 0);
    let mut checks = 0;
    let result = workspace
        .neighborhood_cancellable(
            &graph,
            NodeId(0),
            Direction::Outgoing,
            SearchBudget::default(),
            || {
                checks += 1;
                checks == 3
            },
        )
        .unwrap();
    assert!(result.truncated);
    assert_eq!(result.examined_edges, 256);
    assert_eq!(result.nodes.len(), 257);
    let result = workspace
        .neighborhood(
            &graph,
            NodeId(0),
            Direction::Outgoing,
            SearchBudget::default(),
        )
        .unwrap();
    assert!(!result.truncated);
    assert_eq!(result.nodes.len(), 1025);
}

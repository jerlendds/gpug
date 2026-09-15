use gpug_data::*;
use std::collections::VecDeque;

// Independent edge-list BFS, deliberately unaware of CSR/predecessor storage.
fn distance(
    graph: &GraphSnapshot,
    source: NodeId,
    target: NodeId,
    direction: Direction,
) -> Option<usize> {
    let mut seen = vec![source];
    let mut queue = VecDeque::from([(source, 0)]);
    while let Some((node, depth)) = queue.pop_front() {
        if node == target {
            return Some(depth);
        }
        for edge in graph.edges() {
            let next = if direction != Direction::Incoming && edge.source == node {
                Some(edge.target)
            } else if direction != Direction::Outgoing && edge.target == node {
                Some(edge.source)
            } else {
                None
            };
            if let Some(next) = next {
                if !seen.contains(&next) {
                    seen.push(next);
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }
    None
}

#[test]
fn exhaustive_small_graphs_match_independent_distance_and_real_edge_identity() {
    let ids = [NodeId(u64::MAX), NodeId(17), NodeId(83)];
    let mut workspace = PathWorkspace::new();
    for mask in 0..512 {
        let nodes = ids
            .iter()
            .map(|id| Node::new(*id, WorldRect::default()))
            .collect();
        let mut edges = Vec::new();
        for source in 0..3 {
            for target in 0..3 {
                let index = source * 3 + target;
                if mask & (1 << index) != 0 {
                    edges.push(Edge::new(100 + index as u64, ids[source], ids[target]));
                }
            }
        }
        let graph = GraphSnapshot::new(nodes, edges).unwrap();
        for source in ids {
            for target in ids {
                for direction in [Direction::Outgoing, Direction::Incoming, Direction::Both] {
                    for max_hops in 0..4 {
                        let expected = distance(&graph, source, target, direction)
                            .filter(|distance| *distance <= max_hops);
                        let result = workspace
                            .shortest_path(
                                &graph,
                                source,
                                target,
                                direction,
                                SearchBudget {
                                    max_hops,
                                    max_nodes: 3,
                                    max_edges: 18,
                                },
                            )
                            .unwrap();
                        match expected {
                            None => {
                                assert_eq!(result.status, PathStatus::NotFoundWithinHops);
                                assert!(result.nodes.is_empty() && result.edges.is_empty());
                            }
                            Some(hops) => {
                                assert_eq!(result.status, PathStatus::Found);
                                assert_eq!(result.edges.len(), hops);
                                assert_eq!(result.nodes.len(), hops + 1);
                                assert_eq!(result.nodes.first(), Some(&source));
                                assert_eq!(result.nodes.last(), Some(&target));
                                for (pair, id) in result.nodes.windows(2).zip(result.edges) {
                                    let edge =
                                        graph.edges().iter().find(|edge| edge.id == *id).unwrap();
                                    assert!(
                                        (direction != Direction::Incoming
                                            && edge.source == pair[0]
                                            && edge.target == pair[1])
                                            || (direction != Direction::Outgoing
                                                && edge.target == pair[0]
                                                && edge.source == pair[1])
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn budgets_cancellation_and_reuse_never_report_false_absence_or_partial_paths() {
    let graph = GraphSnapshot::new(
        (0..1025)
            .map(|id| Node::new(id, WorldRect::default()))
            .collect(),
        (1..1025).map(|id| Edge::new(id, 0, id)).collect(),
    )
    .unwrap();
    let mut workspace = PathWorkspace::new();
    let budget = SearchBudget {
        max_hops: 2,
        max_nodes: 2000,
        max_edges: 2000,
    };
    for (budget, reason) in [
        (
            SearchBudget {
                max_nodes: 0,
                ..budget
            },
            PathStop::NodeBudget,
        ),
        (
            SearchBudget {
                max_nodes: 1,
                ..budget
            },
            PathStop::NodeBudget,
        ),
        (
            SearchBudget {
                max_edges: 0,
                ..budget
            },
            PathStop::EdgeBudget,
        ),
    ] {
        let result = workspace
            .shortest_path(&graph, NodeId(0), NodeId(1024), Direction::Outgoing, budget)
            .unwrap();
        assert_eq!(result.status, PathStatus::Stopped(reason));
        assert!(result.nodes.is_empty() && result.edges.is_empty());
    }
    let mut checks = 0;
    let result = workspace
        .shortest_path_cancellable(
            &graph,
            NodeId(0),
            NodeId(1024),
            Direction::Outgoing,
            budget,
            || {
                checks += 1;
                checks == 3
            },
        )
        .unwrap();
    assert_eq!(result.status, PathStatus::Stopped(PathStop::Cancelled));
    assert_eq!(result.examined_edges, 256);
    assert!(result.nodes.is_empty());
    let result = workspace
        .shortest_path(&graph, NodeId(0), NodeId(1024), Direction::Outgoing, budget)
        .unwrap();
    assert_eq!(result.status, PathStatus::Found);
    assert_eq!(result.nodes, &[NodeId(0), NodeId(1024)]);
    assert_eq!(result.edges, &[EdgeId(1024)]);
    assert!(matches!(
        workspace.shortest_path(
            &graph,
            NodeId(0),
            NodeId(u64::MAX),
            Direction::Outgoing,
            budget
        ),
        Err(GraphError::UnknownNode(NodeId(u64::MAX)))
    ));
    // A complete singleton path needs no edge budget.
    let result = workspace
        .shortest_path(
            &graph,
            NodeId(1),
            NodeId(1),
            Direction::Both,
            SearchBudget {
                max_edges: 0,
                ..budget
            },
        )
        .unwrap();
    assert_eq!(result.status, PathStatus::Found);
    assert_eq!(result.nodes, &[NodeId(1)]);
}

#[test]
fn ties_are_deterministic_and_weights_do_not_change_minimum_hops() {
    let mut hidden = Node::new(2, WorldRect::default());
    hidden.hidden = true;
    let mut first = Edge::new(7, 1, 2);
    first.weight = 100.;
    let mut parallel = Edge::new(8, 1, 2);
    parallel.source_handle = Some("other".into());
    let graph = GraphSnapshot::new(
        vec![Node::new(1, WorldRect::default()), hidden],
        vec![first, parallel],
    )
    .unwrap();
    let mut workspace = PathWorkspace::new();
    let result = workspace
        .shortest_path(
            &graph,
            NodeId(1),
            NodeId(2),
            Direction::Outgoing,
            SearchBudget::default(),
        )
        .unwrap();
    assert_eq!(result.edges, &[EdgeId(7)]);
    // Cancellation on reconstruction discards the otherwise valid path.
    let mut checks = 0;
    let result = workspace
        .shortest_path_cancellable(
            &graph,
            NodeId(1),
            NodeId(2),
            Direction::Outgoing,
            SearchBudget::default(),
            || {
                checks += 1;
                checks == 3
            },
        )
        .unwrap();
    assert_eq!(result.status, PathStatus::Stopped(PathStop::Cancelled));
    assert!(result.nodes.is_empty() && result.edges.is_empty());
}

#[test]
fn cancellation_discards_an_already_partially_reconstructed_long_path() {
    let graph = GraphSnapshot::new(
        (0..513)
            .map(|id| Node::new(id, WorldRect::default()))
            .collect(),
        (0..512).map(|id| Edge::new(id, id, id + 1)).collect(),
    )
    .unwrap();
    let budget = SearchBudget {
        max_hops: 512,
        max_nodes: 513,
        max_edges: 512,
    };
    let mut workspace = PathWorkspace::new();
    let mut checks = 0;
    let result = workspace
        .shortest_path_cancellable(
            &graph,
            NodeId(0),
            NodeId(512),
            Direction::Outgoing,
            budget,
            || {
                checks += 1;
                checks == 5
            },
        )
        .unwrap();
    assert_eq!(result.examined_edges, 512);
    assert_eq!(result.visited_nodes, 513);
    assert_eq!(result.status, PathStatus::Stopped(PathStop::Cancelled));
    assert!(result.nodes.is_empty() && result.edges.is_empty());
    let result = workspace
        .shortest_path(&graph, NodeId(0), NodeId(512), Direction::Outgoing, budget)
        .unwrap();
    assert_eq!(result.status, PathStatus::Found);
    assert_eq!(result.edges.len(), 512);
}

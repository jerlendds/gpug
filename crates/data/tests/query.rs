use gpug_data::query::*;
use gpug_data::*;

fn fixture() -> GraphSnapshot {
    let mut nodes = vec![
        Node::new(
            u64::MAX,
            WorldRect {
                x: -2.5,
                y: 0.,
                width: 20.,
                height: 10.,
            },
        ),
        Node::new(7, WorldRect::default()),
        Node::new(9, WorldRect::default()),
    ];
    nodes[0].label = "Ada's 世界".into();
    nodes[0].renderer = "form".into();
    nodes[1].label = "bridge".into();
    nodes[1].hidden = true;
    nodes[2].label = "result".into();
    let a = Edge::new(10, u64::MAX, 7);
    let mut b = Edge::new(11, 7, 9);
    b.weight = 0.;
    b.source_handle = Some(String::new());
    let mut c = Edge::new(12, 9, 9);
    c.source_handle = Some("out".into());
    c.target_handle = Some("in".into());
    GraphSnapshot::new(nodes, vec![a, b, c]).unwrap()
}
fn nodes(text: &str) -> Vec<u64> {
    let mut workspace = QueryWorkspace::new();
    let result = workspace
        .execute(
            &fixture(),
            &Query::parse(text).unwrap(),
            QueryBudget::default(),
        )
        .unwrap();
    assert!(!result.truncated);
    assert!(result.edges.is_empty());
    result.nodes.iter().map(|id| id.0).collect()
}
fn edges(text: &str) -> Vec<u64> {
    let mut workspace = QueryWorkspace::new();
    let result = workspace
        .execute(
            &fixture(),
            &Query::parse(text).unwrap(),
            QueryBudget::default(),
        )
        .unwrap();
    assert!(!result.truncated);
    assert!(result.nodes.is_empty());
    result.edges.iter().map(|id| id.0).collect()
}

#[test]
fn typed_filters_preserve_ids_and_endpoint_joins() {
    assert_eq!(
        nodes("nodes where id = 18446744073709551615"),
        vec![u64::MAX]
    );
    assert_eq!(
        nodes("NODES WHERE label = 'Ada''s 世界' AND x < -2e0"),
        vec![u64::MAX]
    );
    assert_eq!(
        nodes("NODES WHERE renderer CONTAINS 'orm' AND width >= 20 AND height <= 10"),
        vec![u64::MAX]
    );
    assert_eq!(
        nodes("NODES WHERE hidden = TRUE OR in_degree > 1"),
        vec![7, 9]
    );
    assert_eq!(
        edges("EDGES WHERE source.label = 'bridge' AND target.id = 9 AND weight = 0"),
        vec![11]
    );
    assert_eq!(
        edges("EDGES WHERE source = 18446744073709551615 AND target.hidden = TRUE"),
        vec![10]
    );
    assert_eq!(
        edges("EDGES WHERE source.out_degree = 1 AND target.in_degree = 2"),
        vec![11, 12]
    );
}

#[test]
fn boolean_precedence_and_three_valued_null_logic() {
    assert_eq!(
        nodes("NODES WHERE hidden = TRUE OR renderer = 'form' AND x > 0"),
        vec![7]
    );
    assert_eq!(
        nodes("NODES WHERE (hidden = TRUE OR renderer = 'form') AND x > 0"),
        Vec::<u64>::new()
    );
    assert_eq!(nodes("NODES WHERE NOT NOT hidden = TRUE"), vec![7]);
    assert_eq!(edges("EDGES WHERE source_handle IS NULL"), vec![10]);
    assert_eq!(edges("EDGES WHERE source_handle = ''"), vec![11]);
    assert_eq!(edges("EDGES WHERE source_handle IS NOT NULL"), vec![11, 12]);
    assert_eq!(edges("EDGES WHERE NOT source_handle = 'out'"), vec![11]);
    assert_eq!(edges("EDGES WHERE source_handle != 'out'"), vec![11]);
    assert_eq!(
        edges("EDGES WHERE source_handle = 'out' OR weight > 0"),
        vec![10, 12]
    );
    assert_eq!(
        edges("EDGES WHERE NOT (source_handle = 'out' AND weight = 0)"),
        vec![10, 11, 12]
    );
}

#[test]
fn filters_do_not_prune_neighborhood_traversal() {
    let text = "NEIGHBORS 18446744073709551615 OUTGOING HOPS 2 WHERE hidden = FALSE";
    let mut workspace = QueryWorkspace::new();
    let result = workspace
        .execute(&fixture(), &text.parse().unwrap(), QueryBudget::default())
        .unwrap();
    assert_eq!(result.nodes, &[NodeId(u64::MAX), NodeId(9)]);
    assert_eq!(result.depths, &[0, 2]);
    assert_eq!(result.adjacency_examined, 2);
    assert_eq!(nodes("NEIGHBORS 9 INCOMING HOPS 2"), vec![9, 7, u64::MAX]);
    assert_eq!(nodes("NEIGHBORS 7 BOTH HOPS 1 LIMIT 2"), vec![7, 9]);
    assert_eq!(nodes("NEIGHBORS 7 OUTGOING HOPS 0"), vec![7]);
}

#[test]
fn host_budgets_and_query_limits_have_distinct_semantics() {
    let graph = fixture();
    let mut workspace = QueryWorkspace::new();
    let unlimited = Query::parse("NODES").unwrap();
    for cap in 0..3 {
        let result = workspace
            .execute(
                &graph,
                &unlimited,
                QueryBudget {
                    max_results: cap,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(result.nodes.len(), cap);
        assert!(result.truncated);
        let query = Query::parse(&format!("NODES LIMIT {cap}")).unwrap();
        let result = workspace
            .execute(
                &graph,
                &query,
                QueryBudget {
                    max_results: cap,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(result.nodes.len(), cap);
        assert!(!result.truncated);
    }
    let result = workspace
        .execute(
            &graph,
            &Query::parse("NODES WHERE label = 'missing'").unwrap(),
            QueryBudget {
                max_rows: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(result.truncated && result.nodes.is_empty());
    assert_eq!(result.rows_examined, 1);
    let result = workspace
        .execute(
            &graph,
            &Query::parse("NEIGHBORS 7 BOTH HOPS 99").unwrap(),
            QueryBudget {
                max_traversal_edges: 0,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(result.truncated);
    assert_eq!(result.adjacency_examined, 0);
    assert_eq!(result.nodes, &[NodeId(7)]);
    let result = workspace
        .execute(
            &graph,
            &Query::parse("NEIGHBORS 7 BOTH HOPS 99").unwrap(),
            QueryBudget {
                max_traversal_nodes: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(result.truncated);
    assert_eq!(result.nodes, &[NodeId(7)]);
    assert!(workspace
        .execute(
            &graph,
            &Query::parse("NEIGHBORS 123 BOTH HOPS 0 LIMIT 0").unwrap(),
            QueryBudget::default()
        )
        .is_err());
    let empty = GraphSnapshot::new(vec![], vec![]).unwrap();
    let result = workspace
        .execute(
            &empty,
            &unlimited,
            QueryBudget {
                max_rows: 0,
                max_results: 0,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!result.truncated);
}

#[test]
fn errors_are_typed_and_positioned_and_input_complexity_is_bounded() {
    for text in [
        "",
        "NODES WHERE",
        "NODES WHERE id = 18446744073709551616",
        "NODES WHERE id = -1",
        "NODES WHERE id = 1.0",
        "NODES WHERE x = NaN",
        "EDGES WHERE weight = 1e999",
        "NODES WHERE hidden > TRUE",
        "NODES WHERE hidden = 1",
        "NODES WHERE id CONTAINS '1'",
        "EDGES WHERE label = 'x'",
        "NODES WHERE source.label = 'x'",
        "NODES WHERE label = 'unterminated",
        "NODES WHERE label = NULL",
        "NODES LIMIT -1",
        "NODES; EDGES",
        "NEIGHBORS 1 HOPS 2",
        "NODES WHERE (id = 1",
        "NODES WHERE id == 1",
    ] {
        let error = Query::parse(text).expect_err(text);
        assert!(error.offset <= text.len(), "{text}: {error}");
        assert!(!error.message.is_empty());
    }
    let text = "NODES WHERE label = '世界' AND nope = 0";
    assert_eq!(
        Query::parse(text).unwrap_err().offset,
        text.find("nope").unwrap()
    );
    assert!(Query::parse(&format!(
        "NODES WHERE {}id = 1{}",
        "(".repeat(100),
        ")".repeat(100)
    ))
    .is_err());
    assert!(Query::parse(&format!("NODES WHERE {}id = 1", "NOT ".repeat(100))).is_err());
    assert!(Query::parse(&format!("NODES WHERE {}id = 1", "id = 1 OR ".repeat(1000))).is_err());
    assert!(Query::parse(&" ".repeat(65_537)).is_err());
}

#[test]
fn malformed_queries_with_unicode_never_panic() {
    // Input is valid Rust UTF-8; fuzz syntax boundaries, quotes, Unicode, and delimiters.
    let alphabet = ['\'', '(', ')', '!', '=', 'a', '世', ' '];
    for mut code in 0..4096 {
        let mut text = String::from("NODES WHERE ");
        for _ in 0..4 {
            text.push(alphabet[code % alphabet.len()]);
            code /= alphabet.len();
        }
        let _ = Query::parse(&text);
    }
}

#[test]
fn endpoint_queries_match_independent_id_based_oracle() {
    let query =
        Query::parse("EDGES WHERE source.out_degree > 1 AND target.hidden = FALSE").unwrap();
    let mut workspace = QueryWorkspace::new();
    for mask in 0..512u32 {
        let nodes = (0..3)
            .map(|id| {
                let mut n = Node::new(id * 7 + 5, WorldRect::default());
                n.hidden = id == 1;
                n
            })
            .collect();
        let edges = (0..9u64)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| Edge::new(i, (i / 3) * 7 + 5, (i % 3) * 7 + 5))
            .collect();
        let graph = GraphSnapshot::new(nodes, edges).unwrap();
        let expected: Vec<EdgeId> = graph
            .edges()
            .iter()
            .filter(|e| {
                graph
                    .edges()
                    .iter()
                    .filter(|other| other.source == e.source)
                    .count()
                    > 1
                    && !graph
                        .nodes()
                        .iter()
                        .find(|n| n.id == e.target)
                        .unwrap()
                        .hidden
            })
            .map(|e| e.id)
            .collect();
        let result = workspace
            .execute(&graph, &query, QueryBudget::default())
            .unwrap();
        assert_eq!(result.edges, expected, "mask={mask}");
        assert!(!result.truncated);
    }
}

#[test]
fn cancellation_bounds_scan_work_and_workspace_can_be_reused() {
    let graph = GraphSnapshot::new(
        (0..1024)
            .map(|id| Node::new(id, WorldRect::default()))
            .collect(),
        vec![],
    )
    .unwrap();
    let query = Query::parse("NODES").unwrap();
    let mut workspace = QueryWorkspace::new();
    for cancel_after in [0, 1, 2] {
        let mut checks = 0;
        let result = workspace
            .execute_cancellable(&graph, &query, QueryBudget::default(), || {
                checks += 1;
                checks > cancel_after
            })
            .unwrap();
        assert!(result.truncated);
        assert_eq!(result.rows_examined, cancel_after * 256);
        assert_eq!(result.nodes.len(), cancel_after * 256);
    }
    let result = workspace
        .execute(&graph, &query, QueryBudget::default())
        .unwrap();
    assert!(!result.truncated);
    assert_eq!(result.nodes.len(), 1024);
}

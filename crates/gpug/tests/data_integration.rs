use gpug::{analysis as data, EditorModel, GraphOwnership, NodeChange, NodeId, WorldPoint};

fn source() -> data::GraphSnapshot {
    data::GraphSnapshot::new(
        (1..=3)
            .map(|id| data::Node::new(id, data::WorldRect::default()))
            .collect(),
        vec![data::Edge::new(10, 1, 2), data::Edge::new(20, 2, 3)],
    )
    .unwrap()
}

#[test]
fn committed_edits_feed_analysis_and_captures_are_isolated() {
    let mut editor = EditorModel::from_snapshot(&source(), GraphOwnership::Internal).unwrap();
    let before = editor.capture_snapshot().unwrap();
    editor
        .emit_nodes([NodeChange::Position {
            id: NodeId(2),
            position: Some(WorldPoint::new(100., 200.)),
            dragging: Some(false),
        }])
        .unwrap();
    let after = editor.capture_snapshot().unwrap();
    let snapshot = std::thread::spawn(move || after.into_snapshot().unwrap())
        .join()
        .unwrap();
    assert_eq!(snapshot.nodes()[1].bounds.x, 100.);
    assert_eq!(before.into_snapshot().unwrap().nodes()[1].bounds.x, 0.);
    let mut query = data::query::QueryWorkspace::new();
    let result = query
        .execute(
            &snapshot,
            &data::query::Query::parse("NODES WHERE x = 100 AND out_degree = 1").unwrap(),
            data::query::QueryBudget::default(),
        )
        .unwrap();
    assert_eq!(result.nodes, vec![data::NodeId(2)]);
    editor
        .emit_graph_changes(
            [NodeChange::Remove { id: NodeId(2) }],
            [
                gpug::EdgeChange::Remove {
                    id: gpug::EdgeId(10),
                },
                gpug::EdgeChange::Remove {
                    id: gpug::EdgeId(20),
                },
            ],
        )
        .unwrap();
    let deleted = editor.snapshot().unwrap();
    assert_eq!(deleted.nodes().len(), 2);
    assert!(deleted.edges().is_empty());
    assert_eq!(snapshot.edges().len(), 2);
}

#[test]
fn external_proposals_do_not_change_the_analytical_source_until_replaced() {
    let initial = source();
    let mut editor = EditorModel::from_snapshot(&initial, GraphOwnership::External).unwrap();
    editor
        .emit_nodes([NodeChange::Position {
            id: NodeId(1),
            position: Some(WorldPoint::new(42., 0.)),
            dragging: Some(false),
        }])
        .unwrap();
    assert_eq!(editor.snapshot().unwrap().nodes(), initial.nodes());
    let (changes, _) = editor.take_changes();
    assert_eq!(changes.len(), 1);
    let mut host = EditorModel::from_snapshot(&initial, GraphOwnership::Internal).unwrap();
    host.emit_nodes(changes).unwrap();
    editor.replace_snapshot(&host.snapshot().unwrap()).unwrap();
    assert_eq!(editor.snapshot().unwrap().nodes()[0].bounds.x, 42.);
    assert_eq!(editor.store.ownership, GraphOwnership::External);
}

#[test]
fn rejected_snapshot_replacement_leaves_geometry_topology_and_changes_intact() {
    let initial = source();
    let mut editor = EditorModel::from_snapshot(&initial, GraphOwnership::Internal).unwrap();
    let invalid = data::GraphSnapshot::new(
        vec![data::Node::new(
            99,
            data::WorldRect {
                x: f64::MAX,
                ..data::WorldRect::default()
            },
        )],
        vec![],
    )
    .unwrap();
    assert!(editor.replace_snapshot(&invalid).is_err());
    assert_eq!(editor.snapshot().unwrap().nodes(), initial.nodes());
    assert_eq!(editor.snapshot().unwrap().edges(), initial.edges());
    assert_eq!(editor.take_changes(), (vec![], vec![]));
}

#[cfg(feature = "duckdb")]
#[test]
fn edited_graph_persists_reopens_and_rejects_stale_saves() {
    use data::duckdb_store::{DuckDbStore, StoreError, StoreReadLimits};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.duckdb");
    let mut editor = EditorModel::from_snapshot(&source(), GraphOwnership::Internal).unwrap();
    editor
        .emit_nodes([NodeChange::Position {
            id: NodeId(3),
            position: Some(WorldPoint::new(75., 25.)),
            dragging: Some(false),
        }])
        .unwrap();
    let captured = editor.capture_snapshot().unwrap();
    std::thread::spawn(move || {
        let snapshot = captured.into_snapshot().unwrap();
        let mut store = DuckDbStore::open(&path).unwrap();
        let revision = store.save(&snapshot, 0).unwrap();
        assert!(matches!(
            store.save(&source(), 0),
            Err(StoreError::RevisionConflict { .. })
        ));
        drop(store);
        let mut reopened = DuckDbStore::open(path).unwrap();
        let loaded = reopened.load(StoreReadLimits::default()).unwrap();
        assert_eq!(loaded.revision, revision);
        let restored = EditorModel::from_snapshot(&loaded.graph, GraphOwnership::Internal).unwrap();
        assert_eq!(restored.snapshot().unwrap().nodes(), snapshot.nodes());
        assert_eq!(restored.snapshot().unwrap().edges(), snapshot.edges());
    })
    .join()
    .unwrap();
}

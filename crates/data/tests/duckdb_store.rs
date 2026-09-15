#![cfg(feature = "duckdb")]
use gpug_data::duckdb_store::*;
use gpug_data::query::*;
use gpug_data::*;

fn fixture() -> GraphSnapshot {
    let mut a = Node::new(
        u64::MAX,
        WorldRect {
            x: -1.25,
            y: 9.,
            width: 20.,
            height: 30.,
        },
    );
    a.label = "Ada's 世界\0; DROP TABLE gpug_data.nodes; --".into();
    a.renderer = "custom-form".into();
    a.hidden = true;
    let mut b = Node::new(7, WorldRect::default());
    b.label = "bridge".into();
    let mut edges = vec![
        Edge::new(99, u64::MAX, 7),
        Edge::new(0, 7, 7),
        Edge::new(u64::MAX, 7, u64::MAX),
    ];
    edges[1].source_handle = Some(String::new());
    edges[1].weight = 0.;
    edges[2].source_handle = Some("out".into());
    edges[2].target_handle = Some("输入".into());
    GraphSnapshot::new(vec![a, b], edges).unwrap()
}
fn assert_graph(a: &GraphSnapshot, b: &GraphSnapshot) {
    assert_eq!(a.nodes(), b.nodes());
    assert_eq!(a.edges(), b.edges());
    assert_eq!(a.endpoint_columns(), b.endpoint_columns());
}

#[test]
fn transactions_reopen_preserve_fields_and_enforce_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.duckdb");
    let graph = fixture();
    let mut store = DuckDbStore::open(&path).unwrap();
    assert_eq!(store.revision().unwrap(), 0);
    assert!(store
        .load(StoreReadLimits::default())
        .unwrap()
        .graph
        .nodes()
        .is_empty());
    assert_eq!(store.save(&graph, 0).unwrap(), 1);
    let loaded = store.load(StoreReadLimits::default()).unwrap();
    assert_eq!(loaded.revision, 1);
    assert_graph(&loaded.graph, &graph);
    let next = graph.transact([Mutation::RemoveNode(NodeId(7))]).unwrap();
    assert!(matches!(
        store.save(&next, 0),
        Err(StoreError::RevisionConflict {
            expected: 0,
            actual: 1
        })
    ));
    assert_graph(
        &store.load(StoreReadLimits::default()).unwrap().graph,
        &graph,
    );
    assert_eq!(store.save(&next, 1).unwrap(), 2);
    drop(store);
    let mut store = DuckDbStore::open(&path).unwrap();
    let loaded = store.load(StoreReadLimits::default()).unwrap();
    assert_eq!(loaded.revision, 2);
    assert_graph(&loaded.graph, &next);
    assert_eq!(store.save(&graph, 2).unwrap(), 3);
    assert_graph(
        &store.load(StoreReadLimits::default()).unwrap().graph,
        &graph,
    );
}

#[test]
fn database_queries_match_memory_and_bind_literals() {
    let graph = fixture();
    let mut store = DuckDbStore::in_memory().unwrap();
    store.save(&graph, 0).unwrap();
    let mut worker = QueryWorkspace::new();
    for text in [
        "NODES",
        "EDGES",
        "NODES WHERE id = 18446744073709551615",
        "NODES WHERE label CONTAINS '; DROP TABLE gpug_data.nodes; --'",
        "NODES WHERE renderer = 'custom-form' AND hidden = TRUE",
        "NODES WHERE x < 0 AND y = 9 AND width >= 20 AND height <= 30",
        "NODES WHERE in_degree > 1 OR out_degree < 2",
        "NODES WHERE NOT hidden = TRUE",
        "EDGES WHERE id <> 99 AND weight >= 0",
        "EDGES WHERE source = 7 AND target = 7",
        "EDGES WHERE source_handle IS NULL",
        "EDGES WHERE source_handle = ''",
        "EDGES WHERE NOT source_handle = 'out'",
        "EDGES WHERE NOT (source_handle = 'out' AND weight = 0)",
        "EDGES WHERE source.label = 'bridge' AND target.hidden = TRUE",
        "EDGES WHERE source.out_degree = 2 AND target.in_degree >= 1",
        "EDGES WHERE source.id != 7 OR target.renderer = 'default'",
        "NODES LIMIT 1",
        "EDGES LIMIT 0",
    ] {
        let query = Query::parse(text).unwrap();
        let expected = worker
            .execute(&graph, &query, QueryBudget::default())
            .unwrap();
        let actual = store.select(&query, 100).unwrap();
        assert_eq!(actual.nodes, expected.nodes, "{text}");
        assert_eq!(actual.edges, expected.edges, "{text}");
        assert!(!actual.truncated);
        assert_eq!(actual.revision, 1);
    }
    assert_eq!(
        store
            .load(StoreReadLimits::default())
            .unwrap()
            .graph
            .nodes()
            .len(),
        2
    );
    assert!(matches!(
        store.select(&Query::parse("NEIGHBORS 7 BOTH HOPS 1").unwrap(), 10),
        Err(StoreError::UnsupportedQuery)
    ));
}

#[test]
fn read_limits_and_result_caps_do_not_silently_claim_completion() {
    let mut store = DuckDbStore::in_memory().unwrap();
    store.save(&fixture(), 0).unwrap();
    assert!(matches!(
        store.load(StoreReadLimits { nodes: 1, edges: 3 }),
        Err(StoreError::ReadLimit { table: "nodes", .. })
    ));
    assert!(matches!(
        store.load(StoreReadLimits { nodes: 2, edges: 2 }),
        Err(StoreError::ReadLimit { table: "edges", .. })
    ));
    for cap in 0..3 {
        let result = store.select(&Query::parse("EDGES").unwrap(), cap).unwrap();
        assert!(result.truncated);
        assert_eq!(result.edges.len(), cap);
        let result = store
            .select(&Query::parse(&format!("EDGES LIMIT {cap}")).unwrap(), cap)
            .unwrap();
        assert!(!result.truncated);
        assert_eq!(result.edges.len(), cap);
    }
    assert!(
        !store
            .select(&Query::parse("EDGES WHERE id = 123").unwrap(), 0)
            .unwrap()
            .truncated
    );
    assert!(matches!(
        store.select(&Query::parse("EDGES").unwrap(), usize::MAX),
        Err(StoreError::ResultLimitTooLarge)
    ));
    assert_eq!(store.revision().unwrap(), 1);
}

#[test]
fn write_failure_after_deletion_rolls_back_all_tables_and_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.duckdb");
    // Inject an additional database constraint to force a write-time appender error.
    // This deliberately tests failure after the DELETE statements have run.
    let connection = duckdb::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE SCHEMA gpug_data; CREATE TABLE gpug_data.nodes (
        ordinal UBIGINT NOT NULL UNIQUE, id UBIGINT PRIMARY KEY,
        x DOUBLE NOT NULL, y DOUBLE NOT NULL, width DOUBLE NOT NULL, height DOUBLE NOT NULL,
        label VARCHAR NOT NULL CHECK (label <> 'reject'), renderer VARCHAR NOT NULL, hidden BOOLEAN NOT NULL);").unwrap();
    drop(connection);
    let mut store = DuckDbStore::open(&path).unwrap();
    let original = fixture();
    store.save(&original, 0).unwrap();
    let mut rejected = original.nodes()[0].clone();
    rejected.label = "reject".into();
    let rejected = original
        .transact([Mutation::ReplaceNode(rejected)])
        .unwrap();
    assert!(store.save(&rejected, 1).is_err());
    let loaded = store.load(StoreReadLimits::default()).unwrap();
    assert_eq!(loaded.revision, 1);
    assert_graph(&loaded.graph, &original);
}

#[test]
fn independent_connections_reject_stale_writers_and_invalid_saved_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.duckdb");
    let mut first = DuckDbStore::open(&path).unwrap();
    let mut second = DuckDbStore::open(&path).unwrap();
    first.save(&fixture(), 0).unwrap();
    let stale = second.save(&fixture(), 0);
    assert!(
        matches!(&stale, Err(StoreError::RevisionConflict { .. })),
        "{stale:?}"
    );
    second.save(&fixture(), 1).unwrap();
    assert_eq!(first.revision().unwrap(), 2);
    drop(first);
    drop(second);
    let connection = duckdb::Connection::open(&path).unwrap();
    connection
        .execute_batch("UPDATE gpug_data.edges SET target = 123 WHERE id = 99;")
        .unwrap();
    drop(connection);
    let mut store = DuckDbStore::open(&path).unwrap();
    assert!(matches!(
        store.load(StoreReadLimits::default()),
        Err(StoreError::Graph(GraphError::UnknownNode(NodeId(123))))
    ));
    drop(store);
    let connection = duckdb::Connection::open(&path).unwrap();
    connection
        .execute_batch("UPDATE gpug_data.metadata SET schema_version = 2;")
        .unwrap();
    drop(connection);
    assert!(matches!(
        DuckDbStore::open(&path),
        Err(StoreError::SchemaVersion(2))
    ));
}

#[tokio::test]
async fn native_vortex_exports_and_imports_preserve_committed_generations() {
    use gpug_data::vortex_io::*;
    let mut source = DuckDbStore::in_memory().unwrap();
    let mut destination = DuckDbStore::in_memory().unwrap();
    let graph = fixture();
    source.save(&graph, 0).unwrap();
    let io = VortexGraphIo::new();
    let (revision, files) = source
        .export_vortex(&io, StoreReadLimits::default())
        .await
        .unwrap();
    assert_eq!(revision, 1);
    destination
        .import_vortex(&io, files.clone(), ReadLimits::default(), 0)
        .await
        .unwrap();
    assert_graph(
        &destination.load(StoreReadLimits::default()).unwrap().graph,
        &graph,
    );
    assert!(destination
        .import_vortex(&io, files.clone(), ReadLimits::default(), 0)
        .await
        .is_err());
    let corrupt = VortexGraphFiles {
        nodes: files.edges.clone(),
        edges: files.edges,
    };
    assert!(destination
        .import_vortex(&io, corrupt, ReadLimits::default(), 1)
        .await
        .is_err());
    let loaded = destination.load(StoreReadLimits::default()).unwrap();
    assert_eq!(loaded.revision, 1);
    assert_graph(&loaded.graph, &graph);
}

#[test]
fn concurrent_worker_connections_cannot_both_commit_the_same_revision() {
    use std::sync::{Arc, Barrier};
    let mut database = DuckDbStore::in_memory().unwrap();
    database.save(&fixture(), 0).unwrap();
    let mut first = database.try_clone().unwrap();
    let mut second = database.try_clone().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        barrier.wait();
        first.save(&fixture(), 1)
    });
    let second = std::thread::spawn(move || {
        other_barrier.wait();
        second.save(&fixture(), 1)
    });
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "{first:?}, {second:?}"
    );
    assert_eq!(database.revision().unwrap(), 2);
    assert_graph(
        &database.load(StoreReadLimits::default()).unwrap().graph,
        &fixture(),
    );
}

#[test]
fn resource_limits_are_shared_and_conflicting_opens_do_not_reconfigure_workers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resources.duckdb");
    let limits = StoreResources {
        threads: 1,
        memory_limit_bytes: 256 * 1024 * 1024,
    };
    let mut store = DuckDbStore::open_with_resources(&path, limits).unwrap();
    assert_eq!(store.resources(), limits);
    store.save(&fixture(), 0).unwrap();
    let cloned = store.try_clone().unwrap();
    let alias = directory.path().join(".").join("resources.duckdb");
    let reopened = DuckDbStore::open_with_resources(alias, limits).unwrap();
    assert_eq!(cloned.resources(), limits);
    assert_eq!(reopened.revision().unwrap(), 1);
    assert!(
        matches!(DuckDbStore::open(&path), Err(StoreError::ResourceConflict { requested, active })
        if requested == StoreResources::default() && active == limits)
    );
    assert_eq!(store.revision().unwrap(), 1);
    drop(store);
    drop(reopened);
    assert!(matches!(
        DuckDbStore::open(&path),
        Err(StoreError::ResourceConflict { .. })
    ));
    drop(cloned);
    let reopened = DuckDbStore::open(&path).unwrap();
    assert_eq!(reopened.resources(), StoreResources::default());
    assert_eq!(reopened.revision().unwrap(), 1);
}

#[test]
fn invalid_resource_limits_fail_before_creating_a_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.duckdb");
    for resources in [
        StoreResources {
            threads: 0,
            ..Default::default()
        },
        StoreResources {
            memory_limit_bytes: 0,
            ..Default::default()
        },
        StoreResources {
            memory_limit_bytes: u64::MAX,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            DuckDbStore::open_with_resources(&path, resources),
            Err(StoreError::InvalidResources)
        ));
        assert!(!path.exists());
        assert!(matches!(
            DuckDbStore::in_memory_with_resources(resources),
            Err(StoreError::InvalidResources)
        ));
    }
}

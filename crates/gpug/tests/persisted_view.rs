#![cfg(feature = "duckdb")]
use gpug::analysis::{duckdb_store::*, query::*, spatial::*, vortex_io::*, worker::*, *};
use gpug::gpug_bridge::*;
use std::{
    collections::HashSet,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

fn retry<T>(mut action: impl FnMut() -> Result<T, WorkerError>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match action() {
            Ok(value) => return value,
            Err(WorkerError::Busy) if Instant::now() < deadline => std::thread::yield_now(),
            Err(error) => panic!("{error}"),
        }
    }
}

#[tokio::test]
async fn persisted_source_flows_through_worker_query_to_focused_custom_view() {
    let source = GraphSnapshot::new(
        (1000..1064u64)
            .map(|id| {
                let mut node = Node::new(
                    id,
                    WorldRect {
                        x: 0.,
                        y: 0.,
                        width: 180.,
                        height: 100.,
                    },
                );
                node.renderer = "flow-form".into();
                node.label = format!("Step {id} · 世界");
                node
            })
            .collect(),
        (0..64u64)
            .map(|i| {
                let mut edge = Edge::new(u64::MAX - i, 1000 + i, 1000 + (i + 1) % 64);
                edge.source_handle = Some("out".into());
                edge.target_handle = Some(String::new());
                edge.weight = i as f64 / 4.;
                edge
            })
            .collect(),
    )
    .unwrap();
    let io = VortexGraphIo::new();
    let files = io.encode(&source).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("view.duckdb");
    let mut store = DuckDbStore::open(&path).unwrap();
    assert_eq!(
        store
            .import_vortex(&io, files, ReadLimits::default(), 0)
            .await
            .unwrap(),
        1
    );
    drop(store);
    let mut store = DuckDbStore::open(&path).unwrap();
    let selected = store
        .select_cancellable(&Query::parse("NODES WHERE id >= 1016").unwrap(), 64, || {
            false
        })
        .unwrap();
    let loaded = store.load(StoreReadLimits::default()).unwrap();
    assert_eq!(selected.revision, loaded.revision);
    assert_eq!(loaded.graph.nodes(), source.nodes());
    assert_eq!(loaded.graph.edges(), source.edges());
    let filtered = Arc::new(loaded.graph.induced_subgraph(&selected.nodes).unwrap());
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = spawn_graph_query_worker(1, move || {
        let _ = ready_tx.send(());
    })
    .unwrap();
    let client = retry(|| worker.register());
    let stamp = WorkStamp {
        dataset: 42,
        revision: loaded.revision,
    };
    let query = Arc::new(Query::parse("NEIGHBORS 1016 OUTGOING HOPS 63").unwrap());
    let ticket = retry(|| {
        worker
            .submit(
                client,
                stamp,
                GraphQueryTask {
                    graph: Arc::clone(&filtered),
                    query: Arc::clone(&query),
                    budget: QueryBudget::default(),
                },
            )
            .map_err(|error| error.error)
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let completed = retry(|| worker.try_take(client)).expect("completed query");
    assert_eq!(completed.ticket, ticket);
    assert!(retry(|| worker.is_current(ticket)));
    let result = completed.outcome.unwrap().unwrap();
    assert!(!result.truncated);
    assert_eq!(result.nodes, (1016..1064).map(NodeId).collect::<Vec<_>>());
    worker.shutdown_and_join().unwrap();
    let graph = Arc::new(filtered.induced_subgraph(&result.nodes).unwrap());
    let expected_edges = graph
        .edges()
        .iter()
        .map(|edge| edge.id)
        .collect::<HashSet<_>>();
    let index = Arc::new(SpatialIndex::new(graph));
    let plan = Arc::new(
        LodWorkspace::new()
            .prepare_focused(
                index,
                WorldRect {
                    x: -1.,
                    y: -1.,
                    width: 182.,
                    height: 102.,
                },
                LodOptions {
                    zoom: 60.,
                    max_items: 3,
                    max_nodes: 1,
                    max_branches: 1,
                    ..Default::default()
                },
                Some(NodeId(1040)),
            )
            .unwrap(),
    );
    let mut cache = ConnectionCache::new();
    let connections = cache
        .prepare(plan, ConnectionBudget::default(), || false)
        .unwrap();
    let view = to_lod_graph_data(connections, "aggregate").unwrap();
    view.data.compile_edges().unwrap();
    assert_eq!(view.data.nodes.len(), 3);
    let focus = view.data.nodes.last().unwrap();
    assert_eq!(focus.id.0, 1040);
    assert_eq!(focus.node_type, "flow-form");
    assert_eq!(focus.metadata[LABEL_KEY], "Step 1040 · 世界");
    let mut actual_edges = HashSet::new();
    for group in 0..view.connections().groups().len() {
        for edge in view.connections().members(group).unwrap() {
            assert!(actual_edges.insert(edge));
        }
    }
    assert_eq!(actual_edges, expected_edges);
    assert_eq!(actual_edges.len(), 47);
}

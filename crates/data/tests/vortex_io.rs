#![cfg(feature = "vortex")]
use vortex::array::arrays::struct_::StructArrayExt;

use gpug_data::vortex_io::*;
use gpug_data::*;
use vortex::array::arrays::{PrimitiveArray, StructArray};
use vortex::array::expr::{get_item, gt, lit, root, select};
use vortex::array::{ArrayRef, IntoArray, VortexSessionExecute};
use vortex::buffer::ByteBuffer;
use vortex::session::VortexSession;
use vortex::VortexSessionDefault;

fn fixture() -> GraphSnapshot {
    let mut first = Node::new(
        u64::MAX,
        WorldRect {
            x: -100.25,
            y: 0.,
            width: 200.,
            height: 80.,
        },
    );
    first.label = "世界 🦆 'quoted'\0label".into();
    first.renderer = "interactive-form".into();
    first.hidden = true;
    let second = Node::new(2, WorldRect::default());
    let mut a = Edge::new(u64::MAX, u64::MAX, 2);
    a.source_handle = Some(String::new());
    a.weight = 0.;
    let mut b = Edge::new(0, u64::MAX, 2);
    b.target_handle = Some("输入".into());
    GraphSnapshot::new(vec![first, second], vec![a, b]).unwrap()
}

#[tokio::test]
async fn native_files_round_trip_all_fields_and_empty_graph() {
    let io = VortexGraphIo::new();
    for graph in [fixture(), GraphSnapshot::new(vec![], vec![]).unwrap()] {
        let (nodes, edges) = graph_arrays(&graph).unwrap();
        assert_eq!(nodes.dtype(), &node_dtype());
        assert_eq!(edges.dtype(), &edge_dtype());
        let files = io.encode(&graph).await.unwrap();
        assert!(!files.nodes.is_empty() && !files.edges.is_empty());
        // Decode using a fresh session, avoiding accidental reliance on cached arrays.
        let decoded = VortexGraphIo::new()
            .decode(files, ReadLimits::default())
            .await
            .unwrap();
        assert_eq!(decoded.nodes(), graph.nodes());
        assert_eq!(decoded.edges(), graph.edges());
        assert_eq!(decoded.endpoint_columns(), graph.endpoint_columns());
    }
}

#[tokio::test]
async fn filter_and_projection_return_only_requested_rows_and_columns() {
    let io = VortexGraphIo::new();
    let files = io.encode(&fixture()).await.unwrap();
    let projected = io
        .scan(
            files.nodes,
            Some(gt(get_item("id", root()), lit(10u64))),
            Some(select(["id", "renderer"], root())),
        )
        .await
        .unwrap();
    assert_eq!(projected.len(), 1);
    let session = VortexSession::default();
    let mut ctx = session.create_execution_ctx();
    let projected = projected.execute::<StructArray>(&mut ctx).unwrap();
    assert_eq!(projected.names().len(), 2);
    assert!(projected.unmasked_field_by_name_opt("label").is_none());
    let ids = projected
        .unmasked_field_by_name("id")
        .unwrap()
        .clone()
        .execute::<PrimitiveArray>(&mut ctx)
        .unwrap()
        .to_buffer::<u64>();
    assert_eq!(&*ids, &[u64::MAX]);
}

#[tokio::test]
async fn limits_schema_corruption_and_bad_expressions_fail_cleanly() {
    let io = VortexGraphIo::new();
    let files = io.encode(&fixture()).await.unwrap();
    assert!(matches!(
        io.decode(
            files.clone(),
            ReadLimits {
                nodes: 1,
                edges: 10
            }
        )
        .await,
        Err(VortexGraphError::RowLimit { table: "nodes", .. })
    ));
    assert!(io
        .scan(files.nodes.clone(), None, Some(select(["missing"], root())))
        .await
        .is_err());
    let wrong = io
        .encode_table(
            [1u64, 2]
                .into_iter()
                .collect::<PrimitiveArray>()
                .into_array(),
        )
        .await
        .unwrap();
    assert!(matches!(
        io.decode(
            VortexGraphFiles {
                nodes: wrong,
                edges: files.edges.clone()
            },
            ReadLimits::default()
        )
        .await,
        Err(VortexGraphError::Schema { .. })
    ));
    assert!(io
        .decode(
            VortexGraphFiles {
                nodes: ByteBuffer::from(vec![0u8; 16]),
                edges: files.edges
            },
            ReadLimits::default()
        )
        .await
        .is_err());
}

#[test]
fn valid_table_schemas_do_not_bypass_graph_validation() {
    let io = VortexGraphIo::new();
    let graph = fixture();
    let (_, edges) = graph_arrays(&graph).unwrap();
    let different_nodes =
        GraphSnapshot::new(vec![Node::new(9, WorldRect::default())], vec![]).unwrap();
    let (nodes, _) = graph_arrays(&different_nodes).unwrap();
    assert!(matches!(
        io.from_arrays(nodes, edges),
        Err(VortexGraphError::Graph(GraphError::UnknownNode(_)))
    ));
}

#[tokio::test]
async fn compressed_disk_files_reopen_and_scan_independently() {
    let directory = tempfile::tempdir().unwrap();
    let mut nodes = Vec::new();
    for id in 0..4096 {
        let mut node = Node::new(
            id,
            WorldRect {
                x: id as f64,
                y: 10.,
                width: 180.,
                height: 60.,
            },
        );
        node.label =
            "a repeated label with enough text to exercise dictionary/FSST compression".into();
        node.renderer = "custom-node-form".into();
        nodes.push(node);
    }
    let graph = GraphSnapshot::new(
        nodes,
        (0..4095).map(|id| Edge::new(id, id, id + 1)).collect(),
    )
    .unwrap();
    let io = VortexGraphIo::new();
    let files = io.encode(&graph).await.unwrap();
    // Even the numeric node columns alone would require 49 bytes per node.
    // This fixture is deliberately compressible; no general compression ratio is promised.
    assert!(files.nodes.len() < 4096 * 49);
    let node_path = directory.path().join("nodes.vortex");
    let edge_path = directory.path().join("edges.vortex");
    std::fs::write(&node_path, &files.nodes).unwrap();
    std::fs::write(&edge_path, &files.edges).unwrap();
    drop(files);
    drop(io);
    let io = VortexGraphIo::new();
    let recovered = io
        .read_paths(&node_path, &edge_path, ReadLimits::default())
        .await
        .unwrap();
    assert_eq!(recovered.nodes(), graph.nodes());
    assert_eq!(recovered.edges(), graph.edges());
    let positions = io
        .scan_path(
            &node_path,
            Some(gt(get_item("id", root()), lit(4090u64))),
            Some(select(["id", "x", "y"], root())),
        )
        .await
        .unwrap();
    assert_eq!(positions.len(), 5);
    assert!(io
        .read_paths(
            &node_path,
            &edge_path,
            ReadLimits {
                nodes: 4096,
                edges: 1
            }
        )
        .await
        .is_err());
}

fn replace_column(array: ArrayRef, field: &str, replacement: ArrayRef) -> ArrayRef {
    let session = VortexSession::default();
    let array = array
        .execute::<StructArray>(&mut session.create_execution_ctx())
        .unwrap();
    let fields: Vec<(&str, ArrayRef)> = array
        .names()
        .iter()
        .zip(array.iter_unmasked_fields())
        .map(|(name, value)| {
            let name: &str = name.as_ref();
            (
                name,
                if name == field {
                    replacement.clone()
                } else {
                    value.clone()
                },
            )
        })
        .collect();
    StructArray::from_fields(&fields).unwrap().into_array()
}

#[tokio::test]
async fn stored_duplicate_ids_and_invalid_numeric_values_are_rejected() {
    let io = VortexGraphIo::new();
    let (nodes, edges) = graph_arrays(&fixture()).unwrap();
    for (bad_nodes, bad_edges, expected) in [
        (
            replace_column(
                nodes.clone(),
                "id",
                [u64::MAX, u64::MAX]
                    .into_iter()
                    .collect::<PrimitiveArray>()
                    .into_array(),
            ),
            edges.clone(),
            GraphError::DuplicateNode(NodeId(u64::MAX)),
        ),
        (
            replace_column(
                nodes.clone(),
                "x",
                [f64::NAN, 0.]
                    .into_iter()
                    .collect::<PrimitiveArray>()
                    .into_array(),
            ),
            edges.clone(),
            GraphError::InvalidGeometry(NodeId(u64::MAX)),
        ),
        (
            nodes,
            replace_column(
                edges,
                "weight",
                [-1., 1.]
                    .into_iter()
                    .collect::<PrimitiveArray>()
                    .into_array(),
            ),
            GraphError::InvalidWeight(EdgeId(u64::MAX)),
        ),
    ] {
        let files = VortexGraphFiles {
            nodes: io.encode_table(bad_nodes).await.unwrap(),
            edges: io.encode_table(bad_edges).await.unwrap(),
        };
        let result = io.decode(files, ReadLimits::default()).await;
        assert!(matches!(result, Err(VortexGraphError::Graph(error)) if error == expected));
    }
}

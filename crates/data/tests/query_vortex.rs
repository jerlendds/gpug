use gpug_data::query::*;
use gpug_data::vortex_io::*;
use gpug_data::*;
use vortex::array::arrays::struct_::StructArrayExt;
use vortex::array::arrays::{PrimitiveArray, StructArray};
use vortex::array::expr::{root, select};
use vortex::array::VortexSessionExecute;
use vortex::session::VortexSession;
use vortex::VortexSessionDefault;

#[tokio::test]
async fn compiled_predicates_have_the_same_results_in_vortex_and_memory() {
    let mut a = Node::new(u64::MAX, WorldRect::default());
    a.hidden = true;
    a.label = "世界".into();
    let b = Node::new(
        7,
        WorldRect {
            x: -1.,
            y: 2.,
            width: 10.,
            height: 20.,
        },
    );
    let mut edges = vec![
        Edge::new(1, u64::MAX, 7),
        Edge::new(2, 7, 7),
        Edge::new(3, 7, u64::MAX),
    ];
    edges[1].source_handle = Some(String::new());
    edges[1].weight = 0.;
    edges[2].source_handle = Some("out".into());
    let graph = GraphSnapshot::new(vec![a, b], edges).unwrap();
    let io = VortexGraphIo::new();
    let files = io.encode(&graph).await.unwrap();
    let session = VortexSession::default();
    let mut workspace = QueryWorkspace::new();
    for text in [
        "NODES",
        "NODES WHERE id = 18446744073709551615",
        "NODES WHERE hidden = TRUE OR label = 'nothing'",
        "NODES WHERE NOT (x >= 0 AND label != '世界')",
        "NODES WHERE width > 1 AND height <= 20",
        "NODES WHERE label IS NULL",
        "EDGES WHERE source = 7",
        "EDGES WHERE source_handle IS NULL",
        "EDGES WHERE source_handle IS NOT NULL",
        "EDGES WHERE NOT source_handle = 'out'",
        "EDGES WHERE source_handle = '' OR weight >= 1",
        "EDGES WHERE NOT (source_handle = 'out' AND weight = 0)",
    ] {
        let query = Query::parse(text).unwrap();
        let bytes = if query.source() == QuerySource::Edges {
            files.edges.clone()
        } else {
            files.nodes.clone()
        };
        let result = io
            .scan(
                bytes,
                Some(query.vortex_filter().expect(text)),
                Some(select(["id"], root())),
            )
            .await
            .unwrap();
        let mut ctx = session.create_execution_ctx();
        let result = result.execute::<StructArray>(&mut ctx).unwrap();
        let ids = result
            .unmasked_field_by_name("id")
            .unwrap()
            .clone()
            .execute::<PrimitiveArray>(&mut ctx)
            .unwrap()
            .to_buffer::<u64>();
        let expected = workspace
            .execute(&graph, &query, QueryBudget::default())
            .unwrap();
        let expected: Vec<_> = expected
            .nodes
            .iter()
            .map(|n| n.0)
            .chain(expected.edges.iter().map(|e| e.0))
            .collect();
        assert_eq!(&*ids, expected, "{text}");
    }
    for text in [
        "NEIGHBORS 7 BOTH HOPS 1",
        "NODES WHERE out_degree > 0",
        "EDGES WHERE source.label = '世界'",
        "NODES WHERE label CONTAINS '界'",
        "NODES WHERE id > 0 OR out_degree > 0",
    ] {
        assert!(
            Query::parse(text).unwrap().vortex_filter().is_none(),
            "{text}"
        );
    }
}

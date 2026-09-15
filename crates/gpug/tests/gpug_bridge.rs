use gpug::analysis::*;
use gpug::gpug_bridge::*;

fn editor(data: gpug::GraphData) -> gpug::EditorModel {
    gpug::EditorModel::new(data.nodes, data.edges, gpug::GraphOwnership::Internal).unwrap()
}

#[test]
fn renderer_identity_handles_weights_and_hidden_nodes_survive_view_round_trip() {
    let mut a = Node::new(
        u64::MAX,
        WorldRect {
            x: -12.5,
            y: 30.,
            width: 180.,
            height: 90.,
        },
    );
    a.renderer = "flow-form".into();
    a.label = "世界".into();
    let mut b = Node::new(2, WorldRect::default());
    b.hidden = true;
    let mut edge = Edge::new(u64::MAX, u64::MAX, 2);
    edge.source_handle = Some(String::new());
    edge.target_handle = Some("input".into());
    edge.weight = 0.125;
    let graph = GraphSnapshot::new(vec![a, b], vec![edge]).unwrap();
    let view = to_graph_data(&graph).unwrap();
    assert_eq!(view.nodes[0].node_type, "flow-form");
    assert_eq!(view.nodes[0].origin, gpug::WorldPoint::new(0., 0.));
    assert_eq!(view.nodes[0].metadata[LABEL_KEY], "世界");
    let restored = from_editor(&editor(view)).unwrap();
    assert_eq!(graph.nodes(), restored.nodes());
    assert_eq!(graph.edges(), restored.edges());
}

#[test]
fn export_resolves_parent_relative_origins_and_measured_geometry() {
    let parent = gpug::Node::new(1u64, gpug::WorldPoint::new(100., 200.))
        .with_size(gpug::WorldSize::new(80., 60.));
    let child = gpug::Node::new(2u64, gpug::WorldPoint::new(20., 30.))
        .with_size(gpug::WorldSize::new(10., 20.))
        .with_parent(1u64);
    let mut model = editor(gpug::GraphData::new(vec![parent, child], vec![]));
    let snapshot = from_editor(&model).unwrap();
    assert_eq!(
        snapshot.nodes()[0].bounds,
        WorldRect {
            x: 60.,
            y: 170.,
            width: 80.,
            height: 60.
        }
    );
    assert_eq!(
        snapshot.nodes()[1].bounds,
        WorldRect {
            x: 75.,
            y: 190.,
            width: 10.,
            height: 20.
        }
    );
    // Measured runtime geometry is authoritative even before specs are updated.
    let runtime = model.store.runtimes.get_mut(&gpug::NodeId(2)).unwrap();
    runtime.position_absolute = gpug::WorldPoint::new(-50., 40.);
    runtime.measured = gpug::WorldSize::new(50., 60.);
    let snapshot = from_editor(&model).unwrap();
    assert_eq!(
        snapshot.nodes()[1].bounds,
        WorldRect {
            x: -50.,
            y: 40.,
            width: 50.,
            height: 60.
        }
    );
}

#[test]
fn numeric_narrowing_rejects_overflow_and_allows_documented_rounding() {
    for bounds in [
        WorldRect {
            x: f64::MAX,
            ..WorldRect::default()
        },
        WorldRect {
            x: f64::from(f32::MAX),
            width: f64::from(f32::MAX),
            ..WorldRect::default()
        },
    ] {
        let graph = GraphSnapshot::new(vec![Node::new(1, bounds)], vec![]).unwrap();
        assert!(matches!(
            to_graph_data(&graph),
            Err(BridgeError::UnrepresentableGeometry(NodeId(1)))
        ));
    }
    let graph = GraphSnapshot::new(
        vec![Node::new(
            1,
            WorldRect {
                x: 0.1,
                ..WorldRect::default()
            },
        )],
        vec![],
    )
    .unwrap();
    let restored = from_editor(&editor(to_graph_data(&graph).unwrap())).unwrap();
    assert_eq!(restored.nodes()[0].bounds.x, f64::from(0.1f32));
}

#[test]
fn invalid_editor_state_and_reserved_metadata_are_errors() {
    let graph = GraphSnapshot::new(
        vec![Node::new(1, WorldRect::default())],
        vec![Edge::new(2, 1, 1)],
    )
    .unwrap();
    let mut model = editor(to_graph_data(&graph).unwrap());
    model.edges[0].metadata.remove(WEIGHT_KEY);
    assert_eq!(from_editor(&model).unwrap().edges()[0].weight, 1.);
    model.edges[0]
        .metadata
        .insert(WEIGHT_KEY.into(), "invalid".into());
    assert!(matches!(
        from_editor(&model),
        Err(BridgeError::InvalidStoredWeight(EdgeId(2)))
    ));
    model.edges[0]
        .metadata
        .insert(WEIGHT_KEY.into(), "NaN".into());
    assert!(matches!(
        from_editor(&model),
        Err(BridgeError::Data(GraphError::InvalidWeight(EdgeId(2))))
    ));
    model.store.runtimes.clear();
    assert!(matches!(
        from_editor(&model),
        Err(BridgeError::MissingRuntime(NodeId(1)))
    ));
    assert!(to_graph_data(&GraphSnapshot::new(vec![], vec![]).unwrap())
        .unwrap()
        .nodes
        .is_empty());
}

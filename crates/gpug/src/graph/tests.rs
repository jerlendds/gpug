use super::{
    connection_handle_contains, facing_sides, local_to_window, marker_triangle, node_side_point,
    reconnect_endpoint_hit, reconnect_hit_length_pixels, reconnect_path_end_distance,
    reconnect_path_end_hit, reconnecting_edge_id, segment_intersects_bounds, select_content_lod,
    selected_node_bounds, self_loop_path, window_to_local, EdgeKind, GraphDataApi,
};
use crate::WorldSize;
use crate::{
    ConnectionIntent, ConnectionState, Edge, EdgeId, HandleKey, HandleKind, Node, NodeAppearance,
    NodeId, NodeShape, Position, WorldBounds, WorldPoint,
};
use gpui::{point, px, Pixels, Point};

#[test]
fn connection_handles_are_visible_by_default() {
    assert!(super::GraphBuilder::default().show_handles);
}

#[test]
fn step_and_smoothstep_resolve_to_distinct_geometry_kinds() {
    assert_eq!(EdgeKind::from_type("step"), EdgeKind::Step);
    assert_eq!(EdgeKind::from_type("smoothstep"), EdgeKind::SmoothStep);
}

#[test]
fn component_origin_translation_round_trips_window_pointer_coordinates() {
    let origin = point(px(184.0), px(37.0));
    let local = point(px(126.0), px(91.0));
    let window = local_to_window(local, origin);

    assert_eq!(window, point(px(310.0), px(128.0)));
    assert_eq!(window_to_local(window, origin), local);
}

#[test]
fn segment_intersection_counts_crossing_and_touching_bounds() {
    let bounds = WorldBounds::new(WorldPoint::new(10.0, 10.0), WorldSize::new(10.0, 10.0));
    assert!(segment_intersects_bounds(
        WorldPoint::new(0.0, 15.0),
        WorldPoint::new(30.0, 15.0),
        bounds,
    ));
    assert!(segment_intersects_bounds(
        WorldPoint::new(0.0, 10.0),
        WorldPoint::new(10.0, 10.0),
        bounds,
    ));
    assert!(!segment_intersects_bounds(
        WorldPoint::new(0.0, 5.0),
        WorldPoint::new(30.0, 5.0),
        bounds,
    ));
}

#[test]
fn smoothstep_endpoints_use_node_bounding_box_sides() {
    let source = node_side_point(
        WorldPoint::new(0.0, 0.0),
        WorldSize::new(10.0, 8.0),
        Position::Right,
    );
    let target = node_side_point(
        WorldPoint::new(20.0, 0.0),
        WorldSize::new(10.0, 12.0),
        Position::Left,
    );
    let (route, _) =
        crate::connection::smooth_step_path(source, Position::Right, target, Position::Left, 2.0);

    assert_eq!(route.first(), Some(&WorldPoint::new(5.0, 0.0)));
    assert_eq!(route.last(), Some(&WorldPoint::new(15.0, 0.0)));
}

#[test]
fn connection_handles_are_positioned_from_node_bounding_boxes() {
    let center = point(px(100.0), px(80.0));
    let node_size = WorldSize::new(40.0, 12.0);

    let target = super::connection_handle_position(
        center,
        node_size,
        HandleKind::Target,
        Position::Top,
        Position::Right,
        2.0,
    );
    let source = super::connection_handle_position(
        center,
        node_size,
        HandleKind::Source,
        Position::Top,
        Position::Right,
        2.0,
    );

    assert_eq!(target, point(px(100.0), px(67.0)));
    assert_eq!(source, point(px(141.0), px(80.0)));
}

#[test]
fn fixed_pixel_node_handles_follow_the_visible_bounding_box() {
    let zoom = 60.0;
    let appearance = NodeAppearance {
        color: 0,
        radius_pixels: 8.0,
        shape: NodeShape::Square,
    };
    let visible_size = super::connection_geometry_size(WorldSize::new(12.0, 7.0), appearance, zoom);
    let center = point(px(100.0), px(80.0));
    let handle = super::connection_handle_position(
        center,
        visible_size,
        HandleKind::Source,
        Position::Left,
        Position::Right,
        zoom,
    );

    assert_eq!(visible_size, WorldSize::new(16.0 / zoom, 16.0 / zoom));
    assert_eq!(handle, point(px(138.0), px(80.0)));

    let endpoint = super::connection_edge_world_position(
        WorldPoint::new(100.0 / zoom, 80.0 / zoom),
        visible_size,
        Position::Right,
    );
    assert_eq!(
        point(px(endpoint.x * zoom), px(endpoint.y * zoom)),
        point(px(159.0), px(80.0))
    );
}

#[test]
fn default_self_loop_leaves_and_reenters_its_connection_handles() {
    let source = WorldPoint::new(0.0, 5.0);
    let target = WorldPoint::new(0.0, -5.0);
    let path = self_loop_path(source, target);

    assert_eq!(path.first(), Some(&source));
    assert_eq!(path.last(), Some(&target));
    assert!(path.iter().any(|point| point.x >= 10.0));
}

#[test]
fn smoothstep_chooses_the_facing_sides_for_each_layout_direction() {
    assert_eq!(
        facing_sides(WorldPoint::new(0.0, 0.0), WorldPoint::new(20.0, 3.0)),
        (Position::Right, Position::Left)
    );
    assert_eq!(
        facing_sides(WorldPoint::new(0.0, 0.0), WorldPoint::new(-20.0, 3.0)),
        (Position::Left, Position::Right)
    );
    assert_eq!(
        facing_sides(WorldPoint::new(0.0, 0.0), WorldPoint::new(3.0, 20.0)),
        (Position::Bottom, Position::Top)
    );
    assert_eq!(
        facing_sides(WorldPoint::new(0.0, 0.0), WorldPoint::new(3.0, -20.0)),
        (Position::Top, Position::Bottom)
    );
}

/// Level of detail must never remove a node from the frame - it decides
/// how a node is drawn, not whether it is. These assertions are about the
/// flags the paint pass reads, which is where "cheaper" could turn into
/// "missing" by accident.
#[test]
fn content_level_of_detail_promotes_only_nodes_large_enough_to_read() {
    let order = [0usize, 1, 2];
    let hidden = [false, false, false];
    let present = [true, true, true];
    let sizes = [
        WorldSize::new(10.0, 4.0),
        WorldSize::new(10.0, 40.0),
        WorldSize::new(10.0, 40.0),
    ];
    let mut drawn = Vec::new();

    // At this zoom only the two tall nodes clear 18 screen pixels.
    select_content_lod(
        &order, &hidden, &present, None, &sizes, 1.0, 18.0, 16, &mut drawn,
    );
    assert_eq!(drawn, vec![false, true, true]);

    // Zoomed out far enough, nothing is legible and every node falls back
    // to a drawn body.
    select_content_lod(
        &order, &hidden, &present, None, &sizes, 0.1, 18.0, 16, &mut drawn,
    );
    assert_eq!(drawn, vec![false, false, false]);
}

#[test]
fn the_content_budget_binds_from_the_front_of_the_stack() {
    let order = [0usize, 1, 2];
    let hidden = [false, false, false];
    let present = [true, true, true];
    let sizes = [WorldSize::new(10.0, 40.0); 3];
    let mut drawn = Vec::new();

    select_content_lod(
        &order, &hidden, &present, None, &sizes, 1.0, 18.0, 1, &mut drawn,
    );
    assert_eq!(
        drawn,
        vec![false, false, true],
        "the front-most node keeps its element when the budget allows only one"
    );
}

#[test]
fn hidden_and_culled_nodes_are_never_given_elements() {
    let order = [0usize, 1, 2];
    let hidden = [true, false, false];
    let present = [true, true, false];
    let visible = [true, false, true];
    let sizes = [WorldSize::new(10.0, 40.0); 3];
    let mut drawn = Vec::new();

    select_content_lod(
        &order,
        &hidden,
        &present,
        Some(&visible),
        &sizes,
        1.0,
        18.0,
        16,
        &mut drawn,
    );
    assert_eq!(drawn, vec![false, false, false]);
}

#[test]
fn multiple_selected_nodes_get_one_union_bounds() {
    let positions = [
        WorldPoint::new(10.0, 10.0),
        WorldPoint::new(30.0, 20.0),
        WorldPoint::new(100.0, 100.0),
    ];
    let sizes = [
        WorldSize::new(10.0, 8.0),
        WorldSize::new(6.0, 20.0),
        WorldSize::new(50.0, 50.0),
    ];

    let bounds = selected_node_bounds(&[0, 1], &positions, &sizes, &[false; 3]).unwrap();
    assert_eq!(bounds.origin, WorldPoint::new(5.0, 6.0));
    assert_eq!(bounds.size, WorldSize::new(28.0, 24.0));
    assert!(selected_node_bounds(&[0], &positions, &sizes, &[false; 3]).is_none());
    assert!(selected_node_bounds(&[0, 1], &positions, &sizes, &[false, true, false]).is_none());
}

#[test]
fn only_an_edge_being_reconnected_is_hidden_from_painting() {
    let from = HandleKey {
        node: NodeId(1),
        id: None,
        kind: HandleKind::Source,
    };
    let reconnecting = ConnectionState::Connecting {
        from: from.clone(),
        to: None,
        pointer: WorldPoint::ZERO,
        valid: None,
        intent: ConnectionIntent::ReconnectTarget(EdgeId(7)),
    };
    let creating = ConnectionState::Connecting {
        from,
        to: None,
        pointer: WorldPoint::ZERO,
        valid: None,
        intent: ConnectionIntent::Create,
    };

    assert_eq!(reconnecting_edge_id(&reconnecting), Some(EdgeId(7)));
    assert_eq!(reconnecting_edge_id(&creating), None);
    assert_eq!(reconnecting_edge_id(&ConnectionState::Idle), None);
}

#[test]
fn reconnect_target_covers_the_terminal_two_pixels_of_an_edge() {
    let source = point(px(20.0), px(50.0));
    let target = point(px(300.0), px(50.0));

    assert!(reconnect_endpoint_hit(
        point(px(21.9), px(50.0)),
        source,
        target,
        2.0,
        2.0,
    ));
    assert!(!reconnect_endpoint_hit(
        point(px(22.1), px(50.0)),
        source,
        target,
        2.0,
        2.0,
    ));
}

#[test]
fn reconnect_target_extends_one_pixel_beyond_the_visible_stroke() {
    let source = point(px(20.0), px(50.0));
    let target = point(px(300.0), px(50.0));

    // A two-pixel stroke occupies y=49..51, plus one target pixel per side.
    assert!(reconnect_endpoint_hit(
        point(px(21.0), px(52.0)),
        source,
        target,
        2.0,
        2.0,
    ));
    assert!(!reconnect_endpoint_hit(
        point(px(21.0), px(52.1)),
        source,
        target,
        2.0,
        2.0,
    ));
}

#[test]
fn reconnect_target_follows_bends_in_the_painted_path() {
    let path = [
        point(px(20.0), px(20.0)),
        point(px(70.0), px(20.0)),
        point(px(70.0), px(120.0)),
        point(px(220.0), px(120.0)),
    ];

    assert!(reconnect_path_end_hit(
        point(px(21.9), px(20.0)),
        &path,
        true,
        2.0,
        1.0,
    ));
    assert!(!reconnect_path_end_hit(
        point(px(22.1), px(20.0)),
        &path,
        true,
        2.0,
        1.0,
    ));
    assert!(reconnect_path_end_hit(
        point(px(218.1), px(120.0)),
        &path,
        false,
        2.0,
        1.0,
    ));
}

#[test]
fn reconnect_target_length_scales_when_zoomed_in() {
    assert_eq!(reconnect_hit_length_pixels(0.25), 2.0);
    assert_eq!(reconnect_hit_length_pixels(1.0), 2.0);
    assert_eq!(reconnect_hit_length_pixels(2.0), 4.0);
    assert_eq!(reconnect_hit_length_pixels(4.0), 8.0);

    let path = [point(px(0.0), px(10.0)), point(px(1_000.0), px(10.0))];
    assert!(!reconnect_path_end_hit(
        point(px(7.0), px(10.0)),
        &path,
        true,
        2.0,
        1.0,
    ));
    assert!(reconnect_path_end_hit(
        point(px(7.0), px(10.0)),
        &path,
        true,
        2.0,
        4.0,
    ));
}

#[test]
fn connection_handle_hit_test_matches_its_round_visual_bounds() {
    let center = point(px(100.0), px(100.0));
    let diameter = px(179.2);
    let radius = diameter * 0.5;

    assert!(connection_handle_contains(
        point(center.x + radius - px(0.1), center.y),
        center,
        diameter,
    ));
    assert!(!connection_handle_contains(
        point(center.x + radius + px(0.1), center.y),
        center,
        diameter,
    ));
    assert!(!connection_handle_contains(
        point(center.x + radius, center.y + radius),
        center,
        diameter,
    ));
}

#[test]
fn vertically_aligned_connection_handle_accepts_its_visible_center() {
    let node_center = point(px(100.0), px(100.0));
    let handle_center = super::connection_handle_position(
        node_center,
        WorldSize::new(40.0, 12.0),
        HandleKind::Source,
        Position::Top,
        Position::Bottom,
        4.0,
    );

    assert_eq!(handle_center.x, node_center.x);
    assert!(connection_handle_contains(
        handle_center,
        handle_center,
        px(super::CONNECTION_HANDLE_SIZE_WORLD * 4.0),
    ));
}

#[test]
fn overlapping_reconnect_regions_choose_the_nearest_path_endpoint() {
    let path = [point(px(0.0), px(0.0)), point(px(6.0), px(0.0))];
    let click = point(px(3.5), px(0.0));
    let source = reconnect_path_end_distance(click, &path, true, 2.0, 2.0).unwrap();
    let target = reconnect_path_end_distance(click, &path, false, 2.0, 2.0).unwrap();

    assert!(target < source);
    assert_eq!(target, 2.5);
}

#[test]
fn data_api_reads_live_connections_and_merges_node_data() {
    let mut source = Node::new(1_u64, WorldPoint::ZERO);
    source.metadata.insert("text".into(), "hello".into());
    let target = Node::new(2_u64, WorldPoint::ZERO);
    let edge = Edge::new(source.id, target.id).with_id(7_u64);
    let api = GraphDataApi::new();
    api.sync(&[source, target], &[edge]);

    let connections = api.node_connections(NodeId(2), HandleKind::Target);
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].source, NodeId(1));
    assert_eq!(
        api.nodes_data(connections.into_iter().map(|edge| edge.source))[0]
            .metadata
            .get("text")
            .map(String::as_str),
        Some("hello")
    );

    assert!(api.update_node_data(NodeId(1), [("text".into(), "updated".into())]));
    assert_eq!(
        api.node_data(NodeId(1)).unwrap().metadata["text"],
        "updated"
    );
    assert_eq!(api.take_pending().len(), 1);
}

#[test]
fn default_marker_arrow_is_an_isosceles_triangle() {
    let (tip, left, right) =
        marker_triangle(point(px(20.0), px(10.0)), point(px(0.0), px(10.0)), 1.0).unwrap();
    let distance_squared = |a: Point<Pixels>, b: Point<Pixels>| {
        let dx = (a.x - b.x) / px(1.0);
        let dy = (a.y - b.y) / px(1.0);
        dx * dx + dy * dy
    };

    assert!((distance_squared(tip, left) - distance_squared(tip, right)).abs() < 0.001);
    assert_eq!(left.x, right.x);
    assert_eq!((left.y + right.y) / 2.0, tip.y);
}

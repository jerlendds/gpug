use super::*;

fn auto_pan_delta_for_bounds(
    viewport: &Viewport,
    top_left: ViewportPoint,
    bottom_right: ViewportPoint,
    pane: crate::WorldSize,
    margin: f32,
    speed: f32,
) -> ViewportPoint {
    let left = top_left.x <= 0.0;
    let right = bottom_right.x >= pane.width;
    let top = top_left.y <= 0.0;
    let bottom = bottom_right.y >= pane.height;
    let mut delta = ViewportPoint::new(0.0, 0.0);

    if left != right {
        let depth = if left {
            -top_left.x
        } else {
            bottom_right.x - pane.width
        };
        let acceleration = (depth / margin.max(1.0)).clamp(0.15, 1.0);
        let edge_x = if left { top_left.x } else { bottom_right.x };
        delta.x = viewport
            .auto_pan_delta(
                ViewportPoint::new(edge_x, pane.height * 0.5),
                pane,
                margin,
                speed,
            )
            .x
            * acceleration;
    }
    if top != bottom {
        let depth = if top {
            -top_left.y
        } else {
            bottom_right.y - pane.height
        };
        let acceleration = (depth / margin.max(1.0)).clamp(0.15, 1.0);
        let edge_y = if top { top_left.y } else { bottom_right.y };
        delta.y = viewport
            .auto_pan_delta(
                ViewportPoint::new(pane.width * 0.5, edge_y),
                pane,
                margin,
                speed,
            )
            .y
            * acceleration;
    }

    delta
}

impl Graph {
    pub(super) fn node_drag_auto_pan_delta(&self) -> ViewportPoint {
        if !self.auto_pan || self.drag_nodes.is_none() {
            return ViewportPoint::new(0.0, 0.0);
        }
        let Some(pointer) = self
            .pointer
            .as_ref()
            .filter(|pointer| pointer.threshold_exceeded && pointer.is_active())
        else {
            return ViewportPoint::new(0.0, 0.0);
        };
        let Some(pane_size) = self.canvas_size() else {
            return ViewportPoint::new(0.0, 0.0);
        };
        let origin = self.canvas_origin();
        let pane = crate::WorldSize::new(pane_size.width / px(1.0), pane_size.height / px(1.0));
        let speed = self.auto_pan_speed * self.viewport.zoom().clamp(0.25, 2.0);
        let pointer = ViewportPoint::new(
            pointer.current.x - origin.x / px(1.0),
            pointer.current.y - origin.y / px(1.0),
        );
        let zoom = self.viewport.zoom();
        let mut delta = ViewportPoint::new(0.0, 0.0);
        let gesture_node = match self.gestures.gesture {
            Gesture::NodeDrag { node, .. } => Some(node),
            _ => None,
        };
        for (index, offset) in self
            .drag_nodes
            .as_ref()
            .into_iter()
            .flatten()
            .filter(|(index, _)| gesture_node == Some(self.model.nodes[*index].id))
        {
            let node = &self.model.nodes[*index];
            let Some(size) = self
                .model
                .store
                .runtimes
                .get(&node.id)
                .map(|runtime| runtime.measured)
            else {
                continue;
            };
            // Project the node from the live drag pointer instead of reading its
            // stored position. External ownership deliberately leaves that stored
            // snapshot unchanged until the host replaces it, which otherwise keeps
            // auto-pan active after the pointer and node have moved away from an edge.
            let anchor =
                ViewportPoint::new(pointer.x - offset.x * zoom, pointer.y - offset.y * zoom);
            let top_left = ViewportPoint::new(
                anchor.x - size.width * node.origin.x * zoom,
                anchor.y - size.height * node.origin.y * zoom,
            );
            let bottom_right = ViewportPoint::new(
                top_left.x + size.width * zoom,
                top_left.y + size.height * zoom,
            );
            let node_delta = auto_pan_delta_for_bounds(
                &self.viewport,
                top_left,
                bottom_right,
                pane,
                self.auto_pan_margin,
                speed,
            );
            delta = node_delta;
        }
        if delta.x != 0.0 && delta.y != 0.0 {
            if delta.x.abs() >= delta.y.abs() {
                delta.y = 0.0;
            } else {
                delta.x = 0.0;
            }
        }
        delta
    }

    pub(super) fn advance_node_drag_auto_pan(&mut self) -> bool {
        let delta = self.node_drag_auto_pan_delta();
        if delta == ViewportPoint::new(0.0, 0.0) {
            self.auto_pan_edge_since = None;
            return false;
        }
        let edge = (delta.x.signum() as i8, delta.y.signum() as i8);
        let now = std::time::Instant::now();
        let elapsed = match self.auto_pan_edge_since {
            Some((active_edge, since))
                if active_edge == edge
                    && now.duration_since(since) >= Self::DEFAULT_AUTO_PAN_DELAY =>
            {
                now.duration_since(since)
            }
            Some((active_edge, _)) if active_edge == edge => return false,
            _ => {
                self.auto_pan_edge_since = Some((edge, now));
                return false;
            }
        };
        let eased = ((elapsed - Self::DEFAULT_AUTO_PAN_DELAY).as_secs_f32() / 0.5).clamp(0.15, 1.0);
        self.pan_by(point(px(delta.x * eased), px(delta.y * eased)));

        let Some(pointer) = self.pointer.as_ref() else {
            return false;
        };
        let pointer_position = point(px(pointer.current.x), px(pointer.current.y));
        let world = self.screen_to_world(pointer_position);
        let targets = self
            .drag_nodes
            .as_ref()
            .into_iter()
            .flatten()
            .map(|(index, offset)| {
                (
                    self.model.nodes[*index].id,
                    WorldPoint::new(world.x - offset.x, world.y - offset.y),
                )
            })
            .collect::<Vec<_>>();
        self.model.move_nodes(&targets, true);
        self.layout_initialized = false;
        true
    }

    pub(super) fn node_at_screen_position(&self, position: Point<Pixels>) -> Option<usize> {
        let _scope = profile::scope(Phase::Pick);
        let zoom = self.viewport.zoom().max(f32::MIN_POSITIVE);
        let hit_radius = self.renderer.style().hit_radius_pixels;
        let world = self.screen_to_world(position);
        // The pointer's hit region in world units. Nodes without content are
        // hit within a fixed pixel radius; nodes with content are hit within
        // their own body, which can be larger, so the query is widened by the
        // radius and each candidate is then tested exactly.
        let margin = hit_radius / zoom;
        let query = WorldBounds::new(
            WorldPoint::new(world.x - margin, world.y - margin),
            crate::WorldSize::new(margin * 2.0, margin * 2.0),
        );
        let mut candidates = Vec::new();
        self.model
            .store
            .visibility
            .query(&self.model.store.columns, query, &mut candidates);

        let columns = &self.model.store.columns;
        let mut best: Option<(i32, usize)> = None;
        for index in candidates.into_iter().map(|index| index as usize) {
            let node = &self.model.nodes[index];
            if node.hidden {
                continue;
            }
            let center = self.world_to_screen(columns.center(index));
            let (half_width, half_height) = if self
                .content_present
                .get(index)
                .copied()
                .unwrap_or_else(|| self.renderer.has_node_content(node))
            {
                (
                    px(columns.width[index] * zoom * 0.5),
                    px(columns.height[index] * zoom * 0.5),
                )
            } else {
                (px(hit_radius), px(hit_radius))
            };
            if (center.x - position.x).abs() > half_width
                || (center.y - position.y).abs() > half_height
            {
                continue;
            }
            let z = self
                .model
                .store
                .runtimes
                .get(&node.id)
                .map_or(0, |runtime| runtime.z);
            if best.is_none_or(|current| (z, index) > current) {
                best = Some((z, index));
            }
        }
        best.map(|(_, index)| index)
    }

    pub(super) fn node_allows_drag_at_screen_position(
        &self,
        node: &Node,
        position: Point<Pixels>,
    ) -> bool {
        if !node.draggable {
            return false;
        }
        let pointer = self.screen_to_world(position);
        let absolute = self.model.store.node_position_absolute(node);
        let top_left = WorldPoint::new(
            absolute.x - node.size.width * node.origin.x,
            absolute.y - node.size.height * node.origin.y,
        );
        let local = WorldPoint::new(pointer.x - top_left.x, pointer.y - top_left.y);
        node.allows_drag_at(local)
    }

    fn multi_selection_contains_screen_position(&self, position: Point<Pixels>) -> bool {
        let Some(bounds) = selected_node_bounds(
            &self.scene.selected,
            &self.scene.positions,
            &self.scene.node_sizes,
            &self.scene.hidden,
        ) else {
            return false;
        };
        let top_left = self.world_to_screen(bounds.origin);
        let bottom_right = self.world_to_screen(WorldPoint::new(
            bounds.origin.x + bounds.size.width,
            bounds.origin.y + bounds.size.height,
        ));
        let padding = px(MULTI_SELECTION_PADDING_PIXELS);
        position.x >= top_left.x - padding
            && position.x <= bottom_right.x + padding
            && position.y >= top_left.y - padding
            && position.y <= bottom_right.y + padding
    }

    pub(super) fn begin_selected_node_drag(
        &mut self,
        position: Point<Pixels>,
        gesture_node: NodeId,
    ) -> bool {
        let world = self.screen_to_world(position);
        let items = self
            .model
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| self.model.store.node_selected(node) && node.draggable)
            .map(|(index, node)| {
                let absolute = self.model.store.node_position_absolute(node);
                (
                    index,
                    WorldPoint::new(world.x - absolute.x, world.y - absolute.y),
                )
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return false;
        }
        let affected = items
            .iter()
            .map(|(index, _)| self.model.nodes[*index].id)
            .collect();
        self.drag_nodes = Some(items);
        self.auto_pan_edge_since = None;
        self.pointer = Some(PointerController::begin(
            ViewportPoint::new(position.x / px(1.0), position.y / px(1.0)),
            self.gestures.drag_threshold,
            affected,
        ));
        self.gestures.claim(
            GestureOwner::NodeDrag,
            Gesture::NodeDrag {
                node: gesture_node,
                pointer_offset: WorldPoint::ZERO,
            },
        );
        true
    }

    pub(super) fn begin_multi_selection_drag(&mut self, position: Point<Pixels>) -> bool {
        if !self.multi_selection_contains_screen_position(position) {
            return false;
        }
        let Some(node) = self
            .model
            .nodes
            .iter()
            .find(|node| self.model.store.node_selected(node) && node.draggable)
            .map(|node| node.id)
        else {
            return false;
        };
        self.begin_selected_node_drag(position, node)
    }

    pub(super) fn handle_at_screen_position(
        &self,
        position: Point<Pixels>,
        end: bool,
    ) -> Option<(HandleKey, WorldPoint)> {
        let hit = px(CONNECTION_HANDLE_SIZE_WORLD * self.viewport.zoom());
        self.model
            .nodes
            .iter()
            .filter_map(|node| {
                if !node.connectable || node.hidden {
                    return None;
                }
                let center = self.world_to_screen(self.node_center(node));
                let kind = if end {
                    HandleKind::Target
                } else {
                    HandleKind::Source
                };
                if node.connectable_body {
                    let measured = self
                        .model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(node.size, |runtime| runtime.measured);
                    let half_width = px(measured.width * self.viewport.zoom() * 0.5);
                    let half_height = px(measured.height * self.viewport.zoom() * 0.5);
                    let inside = (center.x - position.x).abs() <= half_width
                        && (center.y - position.y).abs() <= half_height;
                    // A whole-body port otherwise consumes every pointer down.
                    // Reserve an explicit custom drag handle for node movement,
                    // while still accepting drops over that region.
                    let reserved_for_drag = !end
                        && node.custom_handle.is_some()
                        && self.node_allows_drag_at_screen_position(node, position);
                    if inside && !reserved_for_drag {
                        return Some((
                            0.0,
                            HandleKey {
                                node: node.id,
                                id: None,
                                kind,
                            },
                            self.node_center(node),
                        ));
                    }
                }
                if !self.show_handles && !self.model.store.node_selected(node) {
                    return None;
                }
                let handle_position = connection_handle_position(
                    center,
                    self.renderer
                        .node_appearance(node, self.viewport.zoom())
                        .radius_pixels,
                    kind,
                    self.target_handle_position,
                    self.source_handle_position,
                    self.viewport.zoom(),
                );
                let dx = (handle_position.x - position.x).abs();
                let dy = (handle_position.y - position.y).abs();
                let center_distance = (center.x - position.x).abs();
                // The fallback handles can be close to the node center at low
                // zoom. Do not let their generous hit box swallow the endpoint
                // hotspot used for reconnecting a selected edge.
                (dx <= hit && dy <= hit && dx < center_distance).then_some((
                    (dx / px(1.0)).powi(2) + (dy / px(1.0)).powi(2),
                    HandleKey {
                        node: node.id,
                        id: None,
                        kind,
                    },
                    self.screen_to_world(handle_position),
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, key, center)| (key, center))
    }

    pub(super) fn is_handle_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.handle_at_screen_position(position, false).is_some()
            || self.handle_at_screen_position(position, true).is_some()
    }

    pub(super) fn reconnect_at_screen_position(
        &self,
        position: Point<Pixels>,
    ) -> Option<(HandleKey, ConnectionIntent)> {
        let hit = px(RECONNECT_HANDLE_SIZE_WORLD * self.viewport.zoom());
        self.model
            .edges
            .iter()
            .filter(|edge| self.model.store.edge_selected(edge) && edge.reconnectable)
            .find_map(|edge| {
                let source = self.model.node(edge.source)?;
                let target = self.model.node(edge.target)?;
                let source_point = self.world_to_screen(self.node_center(source));
                let target_point = self.world_to_screen(self.node_center(target));
                if (source_point.x - position.x).abs() <= hit
                    && (source_point.y - position.y).abs() <= hit
                {
                    Some((
                        HandleKey {
                            node: target.id,
                            id: edge.target_handle.clone().map(Into::into),
                            kind: HandleKind::Target,
                        },
                        ConnectionIntent::ReconnectSource(edge.id),
                    ))
                } else if (target_point.x - position.x).abs() <= hit
                    && (target_point.y - position.y).abs() <= hit
                {
                    Some((
                        HandleKey {
                            node: source.id,
                            id: edge.source_handle.clone().map(Into::into),
                            kind: HandleKind::Source,
                        },
                        ConnectionIntent::ReconnectTarget(edge.id),
                    ))
                } else {
                    None
                }
            })
    }

    pub(super) fn resize_at_screen_position(
        &self,
        position: Point<Pixels>,
    ) -> Option<(usize, crate::ResizeDirection)> {
        if !self.show_resize_handles {
            return None;
        }
        self.model
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                self.model.store.node_selected(node) || node.resize_controls_always_visible
            })
            .find_map(|(index, node)| {
                let hit = px(node.resize_control_hit_radius);
                let center = self.world_to_screen(self.node_center(node));
                node.resize_directions
                    .as_deref()
                    .unwrap_or(&RESIZE_DIRECTIONS)
                    .iter()
                    .copied()
                    .find_map(|direction| {
                        let handle = resize_handle_position(
                            center,
                            node.size,
                            self.viewport.zoom(),
                            direction,
                        );
                        ((handle.x - position.x).abs() <= hit
                            && (handle.y - position.y).abs() <= hit)
                            .then_some((index, direction))
                    })
            })
    }

    pub(super) fn connection_handle(&self, key: HandleKey, center: WorldPoint) -> Handle {
        let position = if key.kind == HandleKind::Target {
            self.target_handle_position
        } else {
            self.source_handle_position
        };
        Handle {
            key,
            bounds: WorldBounds::new(center, crate::WorldSize::new(0.0, 0.0)),
            position,
            connectable_start: true,
            connectable_end: true,
            validation: crate::editor::HandleValidation::Inherit,
        }
    }

    pub(super) fn edge_index_at_screen_position(&self, position: Point<Pixels>) -> Option<usize> {
        let _scope = profile::scope(Phase::Pick);
        let point_x = position.x / px(1.0);
        let point_y = position.y / px(1.0);

        // Hit testing can run between a structural edit and the next render,
        // before `sync` has rebuilt `layout_edges`. When the two agree, an
        // edge's endpoints are node indices and cost two column reads; when
        // they do not, fall back to resolving endpoints by id so a deletion
        // cannot leave this walking mismatched parallel arrays.
        let indexed = self.synced_membership_revision
            == self.model.store.dirty.revisions.membership
            && self.layout_edges.len() == self.model.edges.len()
            && self.model.store.columns.len() == self.model.nodes.len();

        let nearest = |start: Point<Pixels>, end: Point<Pixels>, tolerance: f32| {
            let start_x = start.x / px(1.0);
            let start_y = start.y / px(1.0);
            let end_x = end.x / px(1.0);
            let end_y = end.y / px(1.0);
            // Reject against the segment's bounding box first. Most edges are
            // nowhere near the pointer, and a rectangle test rejects them
            // without the projection, the divide, or the two squares.
            if point_x < start_x.min(end_x) - tolerance
                || point_x > start_x.max(end_x) + tolerance
                || point_y < start_y.min(end_y) - tolerance
                || point_y > start_y.max(end_y) + tolerance
            {
                return false;
            }
            let dx = end_x - start_x;
            let dy = end_y - start_y;
            let length_squared = dx * dx + dy * dy;
            let t = if length_squared > 0.0 {
                (((point_x - start_x) * dx + (point_y - start_y) * dy) / length_squared)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let nearest_x = start_x + t * dx;
            let nearest_y = start_y + t * dy;
            (point_x - nearest_x).powi(2) + (point_y - nearest_y).powi(2) <= tolerance * tolerance
        };

        if indexed {
            let columns = &self.model.store.columns;
            for (edge_index, (edge, layout)) in self
                .model
                .edges
                .iter()
                .zip(self.layout_edges.iter())
                .enumerate()
            {
                let tolerance = edge.interaction_width_for_hit_testing() * 0.5;
                let start = self.world_to_screen(columns.center(layout.source));
                let end = self.world_to_screen(columns.center(layout.target));
                if nearest(start, end, tolerance) {
                    return Some(edge_index);
                }
            }
            return None;
        }

        self.model
            .edges
            .iter()
            .enumerate()
            .find_map(|(edge_index, edge)| {
                let source = self.model.node(edge.source)?;
                let target = self.model.node(edge.target)?;
                let tolerance = edge.interaction_width_for_hit_testing() * 0.5;
                let start = self.world_to_screen(self.node_center(source));
                let end = self.world_to_screen(self.node_center(target));
                nearest(start, end, tolerance).then_some(edge_index)
            })
    }

    fn edge_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.edge_index_at_screen_position(position).is_some()
    }

    pub(super) fn graph_item_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.node_at_screen_position(position).is_some() || self.edge_at_screen_position(position)
    }
}

#[cfg(test)]
mod tests {
    use super::auto_pan_delta_for_bounds;
    use crate::{Viewport, ViewportPoint, WorldSize};

    #[test]
    fn dragged_node_auto_pan_stops_away_from_edges() {
        let viewport = Viewport::default();
        let pane = WorldSize::new(100.0, 80.0);

        assert_ne!(
            auto_pan_delta_for_bounds(
                &viewport,
                ViewportPoint::new(40.0, -1.0),
                ViewportPoint::new(60.0, 19.0),
                pane,
                10.0,
                2.0,
            ),
            ViewportPoint::new(0.0, 0.0)
        );
        assert_eq!(
            auto_pan_delta_for_bounds(
                &viewport,
                ViewportPoint::new(40.0, 20.0),
                ViewportPoint::new(60.0, 40.0),
                pane,
                10.0,
                2.0,
            ),
            ViewportPoint::new(0.0, 0.0)
        );
    }

    #[test]
    fn dragged_node_top_and_bottom_edges_pan_in_opposite_directions() {
        let viewport = Viewport::default();
        let pane = WorldSize::new(100.0, 80.0);
        let top = auto_pan_delta_for_bounds(
            &viewport,
            ViewportPoint::new(40.0, -1.0),
            ViewportPoint::new(60.0, 19.0),
            pane,
            10.0,
            2.0,
        );
        let bottom = auto_pan_delta_for_bounds(
            &viewport,
            ViewportPoint::new(40.0, 61.0),
            ViewportPoint::new(60.0, 81.0),
            pane,
            10.0,
            2.0,
        );

        assert!(top.y > 0.0);
        assert!(bottom.y < 0.0);
    }
}

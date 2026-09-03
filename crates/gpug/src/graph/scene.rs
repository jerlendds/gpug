use super::*;

impl Graph {
    pub(super) fn refresh_scene(&mut self, renderer: &GraphRenderer, zoom: f32) {
        let revisions = self.model.store.dirty.revisions;
        let membership = revisions.membership;
        let specs = (membership, revisions.node_specs, revisions.edge_specs);
        if self.scene.specs != Some(specs) {
            let _scope = profile::scope(Phase::SceneSpecs);
            self.scene.specs = Some(specs);
            self.scene.hidden = self.model.nodes.iter().map(|node| node.hidden).collect();
            self.scene.node_ids = self.model.nodes.iter().map(|node| node.id).collect();
            let columns = &self.model.store.columns;
            self.scene.node_sizes = (0..columns.len())
                .map(|index| columns.size(index))
                .collect();
            self.scene.edge_kinds = self
                .model
                .edges
                .iter()
                .map(|edge| EdgeKind::from_type(&edge.edge_type))
                .collect();
            self.scene.any_curved_edges = self
                .scene
                .edge_kinds
                .iter()
                .any(|kind| *kind != EdgeKind::Straight);
            self.scene.any_edge_labels = self.model.edges.iter().any(|edge| edge.label.is_some());
            self.scene.edge_ids = self.model.edges.iter().map(|edge| edge.id).collect();
            self.scene.edge_markers = self
                .model
                .edges
                .iter()
                .map(|edge| (edge.marker_start.clone(), edge.marker_end.clone()))
                .collect();
            // Resize configuration is part of a node's specification, so it
            // belongs with the other spec-derived columns. Rebuilt here, these
            // are four `Rc` clones per frame instead of four fresh vectors.
            self.scene.resize_directions = self
                .model
                .nodes
                .iter()
                .map(|node| {
                    node.resize_directions
                        .as_deref()
                        .map(|directions| Rc::from(directions))
                })
                .collect();
            self.scene.show_resize_controls = self
                .model
                .nodes
                .iter()
                .map(|node| node.show_resize_controls)
                .collect();
            self.scene.resize_controls_always_visible = self
                .model
                .nodes
                .iter()
                .map(|node| node.resize_controls_always_visible)
                .collect();
            self.scene.resize_control_colors = self
                .model
                .nodes
                .iter()
                .map(|node| node.resize_control_color)
                .collect();
            let mut present = std::mem::take(&mut self.content_present);
            let slot = Rc::make_mut(&mut present);
            slot.clear();
            slot.extend(
                self.model
                    .nodes
                    .iter()
                    .map(|node| renderer.has_node_content(node)),
            );
            self.content_present = present;
        }

        let motion = (membership, revisions.nodes, revisions.edges);
        if self.scene.motion != Some(motion) {
            let _scope = profile::scope(Phase::SceneMotion);
            self.scene.motion = Some(motion);
            let mut positions_buffer = std::mem::take(&mut self.scene.positions);
            {
                let slot = Rc::make_mut(&mut positions_buffer);
                slot.clear();
                let columns = &self.model.store.columns;
                slot.extend((0..columns.len()).map(|index| columns.center(index)));
            }
            self.scene.positions = positions_buffer;
            let positions = self.scene.positions.clone();
            let kinds = self.scene.edge_kinds.clone();
            let mut cache = std::mem::take(&mut self.edge_geometry_cache);
            let mut geometry_buffer = std::mem::take(&mut self.scene.edge_geometries);
            let geometries = Rc::make_mut(&mut geometry_buffer);
            geometries.clear();
            // A graph of straight edges needs no sampled geometry at all. An
            // empty column means "every edge is its two endpoints", which the
            // paint passes already fall back to, so this skips a pass over
            // every edge on every frame that moves a node.
            if !self.scene.any_curved_edges {
                self.scene.edge_geometries = geometry_buffer;
                self.edge_geometry_cache = cache;
            } else {
                geometries.reserve(self.model.edges.len());
                for (index, (edge, layout)) in self
                    .model
                    .edges
                    .iter()
                    .zip(self.layout_edges.iter())
                    .enumerate()
                {
                    let kind = kinds.get(index).copied().unwrap_or(EdgeKind::Straight);
                    if kind == EdgeKind::Straight {
                        geometries.push(None);
                        continue;
                    }
                    let stamp = (
                        *self.model.store.edge_revisions.get(&edge.id).unwrap_or(&0),
                        self.model
                            .store
                            .runtimes
                            .get(&edge.source)
                            .map_or(0, |runtime| runtime.revision),
                        self.model
                            .store
                            .runtimes
                            .get(&edge.target)
                            .map_or(0, |runtime| runtime.revision),
                    );
                    let a = positions[layout.source];
                    let b = positions[layout.target];
                    geometries.push(Some(cache.get_or_insert_with(
                        edge.id,
                        stamp,
                        || match kind {
                            EdgeKind::Bezier => {
                                let (curve, _) = crate::connection::bezier_path(
                                    a,
                                    self.source_handle_position,
                                    b,
                                    self.target_handle_position,
                                    0.25,
                                );
                                (0..=12)
                                    .map(|index| {
                                        let t = index as f32 / 12.0;
                                        let u = 1.0 - t;
                                        WorldPoint::new(
                                            u * u * u * curve[0].x
                                                + 3.0 * u * u * t * curve[1].x
                                                + 3.0 * u * t * t * curve[2].x
                                                + t * t * t * curve[3].x,
                                            u * u * u * curve[0].y
                                                + 3.0 * u * u * t * curve[1].y
                                                + 3.0 * u * t * t * curve[2].y
                                                + t * t * t * curve[3].y,
                                        )
                                    })
                                    .collect()
                            }
                            EdgeKind::SmoothStep => {
                                let (source_side, target_side) = facing_sides(a, b);
                                let a = node_side_point(
                                    a,
                                    self.scene.node_sizes[layout.source],
                                    source_side,
                                );
                                let b = node_side_point(
                                    b,
                                    self.scene.node_sizes[layout.target],
                                    target_side,
                                );
                                crate::connection::smooth_step_path(
                                    a,
                                    source_side,
                                    b,
                                    target_side,
                                    2.0,
                                )
                                .0
                            }
                            EdgeKind::Custom => {
                                renderer.edge_path(edge, a, b).unwrap_or_else(|| vec![a, b])
                            }
                            EdgeKind::Straight => vec![a, b],
                        },
                    )));
                }
                self.edge_geometry_cache = cache;
                self.scene.edge_geometries = geometry_buffer;
            }
        }

        // Custom renderers are handed the whole node or edge, selection flag
        // included, so a selection change can repaint them even when nothing
        // was added or moved.
        let appearance = AppearanceRevision {
            membership,
            node_specs: revisions.node_specs,
            edge_specs: revisions.edge_specs,
            selection: revisions.selection,
            style: self.style_revision,
            zoom_bits: zoom.to_bits(),
        };
        if self.scene.appearance != Some(appearance) {
            let _scope = profile::scope(Phase::SceneAppearance);
            self.scene.appearance = Some(appearance);
            self.scene.node_appearances = self
                .model
                .nodes
                .iter()
                .map(|node| renderer.node_appearance(node, zoom))
                .collect();
            self.scene.edge_appearances = self
                .model
                .edges
                .iter()
                .map(|edge| renderer.edge_appearance(edge))
                .collect();
        }

        let selection = (membership, revisions.selection);
        if self.scene.selection != Some(selection) {
            let _scope = profile::scope(Phase::SceneSelection);
            self.scene.selection = Some(selection);
            self.scene.selected = self
                .model
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| self.model.store.node_selected(node).then_some(index))
                .collect();
            let mut node_order = (0..self.model.nodes.len()).collect::<Vec<_>>();
            node_order.sort_by_key(|&index| {
                let node = &self.model.nodes[index];
                (
                    self.model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(0, |runtime| runtime.z),
                    index,
                )
            });
            self.scene.node_order = node_order.into();
            self.scene.selected_edges = self
                .model
                .edges
                .iter()
                .zip(self.layout_edges.iter())
                .enumerate()
                .filter_map(|(index, (edge, layout))| {
                    self.model
                        .store
                        .edge_selected(edge)
                        .then_some((index, *layout))
                })
                .collect();
        }
    }
}

pub(super) const RESIZE_DIRECTIONS: [crate::ResizeDirection; 8] = [
    crate::ResizeDirection::NorthWest,
    crate::ResizeDirection::North,
    crate::ResizeDirection::NorthEast,
    crate::ResizeDirection::East,
    crate::ResizeDirection::SouthEast,
    crate::ResizeDirection::South,
    crate::ResizeDirection::SouthWest,
    crate::ResizeDirection::West,
];

/// Selects the nodes that render element content this frame.
///
/// Nodes must be legible at the current zoom and fit within the element
/// budget. Nodes rejected here still receive their cheaper painted body.
#[allow(clippy::too_many_arguments)]
pub(super) fn select_content_lod(
    order: &[usize],
    hidden: &[bool],
    present: &[bool],
    visible: Option<&[bool]>,
    sizes: &[crate::WorldSize],
    zoom: f32,
    min_pixels: f32,
    budget: usize,
    drawn: &mut Vec<bool>,
) {
    drawn.clear();
    drawn.resize(present.len(), false);
    let mut remaining = budget;
    for index in order.iter().rev().copied() {
        if remaining == 0 {
            break;
        }
        if index >= present.len()
            || hidden[index]
            || !present[index]
            || visible.is_some_and(|visible| !visible[index])
            || sizes[index].height * zoom < min_pixels
        {
            continue;
        }
        drawn[index] = true;
        remaining -= 1;
    }
}

pub(super) fn resize_handle_position(
    center: Point<Pixels>,
    node_size: crate::WorldSize,
    zoom: f32,
    direction: crate::ResizeDirection,
) -> Point<Pixels> {
    let x = match direction {
        crate::ResizeDirection::NorthWest
        | crate::ResizeDirection::West
        | crate::ResizeDirection::SouthWest => -1.0,
        crate::ResizeDirection::NorthEast
        | crate::ResizeDirection::East
        | crate::ResizeDirection::SouthEast => 1.0,
        _ => 0.0,
    };
    let y = match direction {
        crate::ResizeDirection::NorthWest
        | crate::ResizeDirection::North
        | crate::ResizeDirection::NorthEast => -1.0,
        crate::ResizeDirection::SouthWest
        | crate::ResizeDirection::South
        | crate::ResizeDirection::SouthEast => 1.0,
        _ => 0.0,
    };
    point(
        center.x + px(x * node_size.width * 0.5 * zoom),
        center.y + px(y * node_size.height * 0.5 * zoom),
    )
}

pub(super) fn connection_handle_position(
    center: Point<Pixels>,
    radius_pixels: f32,
    kind: HandleKind,
    target_position: Position,
    source_position: Position,
    zoom: f32,
) -> Point<Pixels> {
    let offset = px(radius_pixels + CONNECTION_HANDLE_GAP_WORLD * zoom);
    match if kind == HandleKind::Target {
        target_position
    } else {
        source_position
    } {
        Position::Left => point(center.x - offset, center.y),
        Position::Top => point(center.x, center.y - offset),
        Position::Right => point(center.x + offset, center.y),
        Position::Bottom => point(center.x, center.y + offset),
    }
}

pub(super) fn world_bounds(nodes: &[Node], store: &EditorStore) -> Option<WorldBounds> {
    let mut visible = nodes.iter().filter(|node| !node.hidden);
    let first = visible.next()?;
    let first = store.runtimes.get(&first.id)?.bounds();
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        first.origin.x,
        first.origin.x + first.size.width,
        first.origin.y,
        first.origin.y + first.size.height,
    );
    for node in visible {
        let bounds = store.runtimes.get(&node.id)?.bounds();
        min_x = min_x.min(bounds.origin.x);
        max_x = max_x.max(bounds.origin.x + bounds.size.width);
        min_y = min_y.min(bounds.origin.y);
        max_y = max_y.max(bounds.origin.y + bounds.size.height);
    }
    Some(WorldBounds::new(
        WorldPoint::new(min_x, min_y),
        crate::WorldSize::new(max_x - min_x, max_y - min_y),
    ))
}

pub(super) fn selected_node_bounds(
    selected: &[usize],
    positions: &[WorldPoint],
    sizes: &[crate::WorldSize],
    hidden: &[bool],
) -> Option<WorldBounds> {
    if selected.len() < 2 {
        return None;
    }
    let mut bounds = selected
        .iter()
        .copied()
        .filter(|&index| !hidden[index])
        .map(|index| {
            let position = positions[index];
            let size = sizes[index];
            WorldBounds::new(
                WorldPoint::new(
                    position.x - size.width * 0.5,
                    position.y - size.height * 0.5,
                ),
                size,
            )
        });
    let first = bounds.next()?;
    let second = bounds.next()?;
    let (mut min_x, mut min_y) = (first.origin.x, first.origin.y);
    let (mut max_x, mut max_y) = (
        first.origin.x + first.size.width,
        first.origin.y + first.size.height,
    );
    for bounds in std::iter::once(second).chain(bounds) {
        min_x = min_x.min(bounds.origin.x);
        min_y = min_y.min(bounds.origin.y);
        max_x = max_x.max(bounds.origin.x + bounds.size.width);
        max_y = max_y.max(bounds.origin.y + bounds.size.height);
    }
    Some(WorldBounds::new(
        WorldPoint::new(min_x, min_y),
        crate::WorldSize::new(max_x - min_x, max_y - min_y),
    ))
}

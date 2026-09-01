//! Framework-neutral graph editor model and interaction contracts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{
    Connection, ConnectionIntent, DeleteDecision, DeleteSet, DirtySet, DirtyTracker, Edge,
    GraphDataError, GraphHost, Node, NodeId, VisibilityIndex, WorldBounds, WorldPoint, WorldSize,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EdgeId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HandleKind {
    Source,
    Target,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Position {
    Left,
    Top,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionMode {
    Strict,
    Loose,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HandleValidation {
    #[default]
    Inherit,
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Full,
    Partial,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GraphOwnership {
    #[default]
    Internal,
    External,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HandleKey {
    pub node: NodeId,
    pub id: Option<Arc<str>>,
    pub kind: HandleKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Handle {
    pub key: HandleKey,
    pub bounds: WorldBounds,
    pub position: Position,
    pub connectable_start: bool,
    pub connectable_end: bool,
    pub validation: HandleValidation,
}

impl Handle {
    pub fn center(&self, node_origin: WorldPoint) -> WorldPoint {
        WorldPoint::new(
            node_origin.x + self.bounds.origin.x + self.bounds.size.width * 0.5,
            node_origin.y + self.bounds.origin.y + self.bounds.size.height * 0.5,
        )
    }
}

#[derive(Clone, Debug)]
pub struct NodeRuntime {
    pub position_absolute: WorldPoint,
    pub measured: WorldSize,
    pub handles: Vec<Handle>,
    pub z: i32,
    z_before_selection: Option<i32>,
    pub dragging: bool,
    pub resizing: bool,
    pub revision: u64,
}

impl NodeRuntime {
    pub fn from_node(node: &Node) -> Self {
        Self {
            position_absolute: node.position,
            measured: node.size,
            handles: Vec::new(),
            z: 0,
            z_before_selection: None,
            dragging: false,
            resizing: false,
            revision: 0,
        }
    }

    pub fn bounds(&self) -> WorldBounds {
        WorldBounds::new(self.position_absolute, self.measured)
    }
}

/// Sentinel for "this node has no parent" in [`NodeColumns::parent`].
pub const NO_PARENT: u32 = u32::MAX;

/// Dense, index-addressed node geometry: the graph kernel's view of the state
/// the frame path reads.
///
/// [`EditorStore::runtimes`] stays the id-addressed store that the rich editor
/// API is written against. These columns hold the same geometry one entry per
/// node index, contiguous and numeric, because a frame touches every node in
/// order: a layout step, a cull, and a scene rebuild each want a sequential
/// scan, and a hash lookup per node per column turns that scan into one
/// pointer chase per value.
///
/// The two representations are deliberately separate. The rich one absorbs
/// arbitrary application data and changes shape as the editor grows; this one
/// stays numeric so it can be scanned, chunked, vectorized, and eventually
/// handed to the GPU unchanged.
#[derive(Clone, Debug, Default)]
pub struct NodeColumns {
    /// Absolute top-left corner in world units.
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    /// Measured size in world units.
    pub width: Vec<f32>,
    pub height: Vec<f32>,
    /// Which point inside the node its position refers to, in 0..1.
    pub origin_x: Vec<f32>,
    pub origin_y: Vec<f32>,
    /// Parent node index, or [`NO_PARENT`]. An index removes the hash lookup
    /// a layout frame would otherwise pay per node to resolve hierarchy.
    pub parent: Vec<u32>,
    pub hidden: Vec<bool>,
    /// Advances on every write, so a consumer that caches something derived
    /// from the columns can tell whether it is still current.
    revision: u64,
    /// Whether the graph is a forest of roots only. Computed when the columns
    /// are rebuilt so a layout frame branches once instead of per node.
    flat: bool,
}

impl NodeColumns {
    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[inline]
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// True when no node has a parent, which lets a layout frame skip
    /// hierarchy resolution for the whole graph rather than per node.
    pub fn is_flat(&self) -> bool {
        self.flat
    }

    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.width.clear();
        self.height.clear();
        self.origin_x.clear();
        self.origin_y.clear();
        self.parent.clear();
        self.hidden.clear();
    }

    fn push(&mut self, node: &Node, absolute: WorldPoint, size: WorldSize, parent: u32) {
        self.x.push(absolute.x);
        self.y.push(absolute.y);
        self.width.push(size.width);
        self.height.push(size.height);
        self.origin_x.push(node.origin.x);
        self.origin_y.push(node.origin.y);
        self.parent.push(parent);
        self.hidden.push(node.hidden);
    }

    /// The node's visual center in world units.
    #[inline]
    pub fn center(&self, index: usize) -> WorldPoint {
        WorldPoint::new(
            self.x[index] + self.width[index] * 0.5,
            self.y[index] + self.height[index] * 0.5,
        )
    }

    /// The point the node's position refers to, in world units.
    #[inline]
    pub fn anchor(&self, index: usize) -> WorldPoint {
        WorldPoint::new(
            self.x[index] + self.width[index] * self.origin_x[index],
            self.y[index] + self.height[index] * self.origin_y[index],
        )
    }

    #[inline]
    pub fn size(&self, index: usize) -> WorldSize {
        WorldSize::new(self.width[index], self.height[index])
    }

    #[inline]
    pub fn bounds(&self, index: usize) -> WorldBounds {
        WorldBounds::new(
            WorldPoint::new(self.x[index], self.y[index]),
            self.size(index),
        )
    }

    #[cfg(test)]
    pub(crate) fn push_for_test(
        &mut self,
        node: &Node,
        absolute: WorldPoint,
        size: WorldSize,
        parent: u32,
    ) {
        self.push(node, absolute, size, parent);
        self.flat = self.parent.iter().all(|parent| *parent == NO_PARENT);
    }

    #[inline]
    pub fn set_position(&mut self, index: usize, absolute: WorldPoint) {
        self.x[index] = absolute.x;
        self.y[index] = absolute.y;
        self.touch();
    }

    #[inline]
    pub fn set_size(&mut self, index: usize, size: WorldSize) {
        self.width[index] = size.width;
        self.height[index] = size.height;
        self.touch();
    }
}

impl EditorStore {
    fn raise_selected_node(&mut self, id: NodeId) -> bool {
        let next = self
            .runtimes
            .values()
            .map(|runtime| runtime.z)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let Some(runtime) = self.runtimes.get_mut(&id) else {
            return false;
        };
        if runtime.z_before_selection.is_none() {
            runtime.z_before_selection = Some(runtime.z);
        }
        if runtime.z == next {
            return false;
        }
        runtime.z = next;
        runtime.revision = runtime.revision.wrapping_add(1);
        self.dirty.mark_node(id);
        true
    }

    fn restore_deselected_node(&mut self, id: NodeId) -> bool {
        let Some(runtime) = self.runtimes.get_mut(&id) else {
            return false;
        };
        let Some(previous) = runtime.z_before_selection.take() else {
            return false;
        };
        runtime.z = previous;
        runtime.revision = runtime.revision.wrapping_add(1);
        self.dirty.mark_node(id);
        true
    }

    fn apply_selection_stacking(&mut self, changes: &[NodeChange]) {
        for change in changes {
            if let NodeChange::Select { id, selected } = change {
                if *selected {
                    self.raise_selected_node(*id);
                } else {
                    self.restore_deselected_node(*id);
                }
            }
        }
    }

    /// The node's position anchor in world coordinates.
    pub fn node_position_absolute(&self, node: &Node) -> WorldPoint {
        self.runtimes
            .get(&node.id)
            .map_or(node.position, |runtime| {
                WorldPoint::new(
                    runtime.position_absolute.x + runtime.measured.width * node.origin.x,
                    runtime.position_absolute.y + runtime.measured.height * node.origin.y,
                )
            })
    }

    /// The node's visual center in world coordinates.
    pub fn node_center_absolute(&self, node: &Node) -> WorldPoint {
        self.runtimes.get(&node.id).map_or_else(
            || node_center(node),
            |runtime| {
                WorldPoint::new(
                    runtime.position_absolute.x + runtime.measured.width * 0.5,
                    runtime.position_absolute.y + runtime.measured.height * 0.5,
                )
            },
        )
    }

    /// Converts a world-space position anchor back to the coordinate system
    /// used by the node specification.
    pub fn position_relative_to_parent(&self, node: &Node, absolute: WorldPoint) -> WorldPoint {
        let parent_origin = node
            .parent_id
            .and_then(|id| self.runtimes.get(&id))
            .map(|runtime| runtime.position_absolute)
            .unwrap_or(WorldPoint::ZERO);
        WorldPoint::new(absolute.x - parent_origin.x, absolute.y - parent_origin.y)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeChange {
    Add {
        index: Option<usize>,
        item: Node,
    },
    Remove {
        id: NodeId,
    },
    Replace {
        id: NodeId,
        item: Node,
    },
    Select {
        id: NodeId,
        selected: bool,
    },
    Position {
        id: NodeId,
        position: Option<WorldPoint>,
        dragging: Option<bool>,
    },
    Dimensions {
        id: NodeId,
        size: Option<WorldSize>,
        resizing: Option<bool>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EdgeChange {
    Add { index: Option<usize>, item: Edge },
    Remove { id: EdgeId },
    Replace { id: EdgeId, item: Edge },
    Select { id: EdgeId, selected: bool },
}
pub type NodeChangeMiddleware = Arc<dyn Fn(&mut Vec<NodeChange>) + Send + Sync>;
pub type EdgeChangeMiddleware = Arc<dyn Fn(&mut Vec<EdgeChange>) + Send + Sync>;

fn apply_node_changes_unchecked(nodes: &mut Vec<Node>, changes: &[NodeChange]) {
    let structural: HashMap<_, _> = changes
        .iter()
        .filter_map(|change| match change {
            NodeChange::Remove { id } => Some((*id, change)),
            NodeChange::Replace { id, .. } => Some((*id, change)),
            _ => None,
        })
        .collect();
    nodes.retain_mut(|node| {
        if let Some(change) = structural.get(&node.id) {
            match change {
                NodeChange::Remove { .. } => return false,
                NodeChange::Replace { item, .. } => {
                    *node = item.clone();
                    return true;
                }
                _ => unreachable!(),
            }
        }
        for change in changes {
            match change {
                NodeChange::Select { id, selected } if *id == node.id => node.selected = *selected,
                NodeChange::Position {
                    id,
                    position: Some(position),
                    ..
                } if *id == node.id => node.position = *position,
                NodeChange::Dimensions {
                    id,
                    size: Some(size),
                    ..
                } if *id == node.id => node.size = *size,
                _ => {}
            }
        }
        true
    });
    for change in changes {
        if let NodeChange::Add { index, item } = change {
            nodes.insert(index.unwrap_or(nodes.len()).min(nodes.len()), item.clone());
        }
    }
}

fn apply_edge_changes_unchecked(edges: &mut Vec<Edge>, changes: &[EdgeChange]) {
    let structural: HashMap<_, _> = changes
        .iter()
        .filter_map(|change| match change {
            EdgeChange::Remove { id } => Some((*id, change)),
            EdgeChange::Replace { id, .. } => Some((*id, change)),
            _ => None,
        })
        .collect();
    edges.retain_mut(|edge| {
        if let Some(change) = structural.get(&edge.id) {
            match change {
                EdgeChange::Remove { .. } => return false,
                EdgeChange::Replace { item, .. } => {
                    *edge = item.clone();
                    return true;
                }
                _ => unreachable!(),
            }
        }
        for change in changes {
            if let EdgeChange::Select { id, selected } = change {
                if *id == edge.id {
                    edge.selected = *selected;
                }
            }
        }
        true
    });
    for change in changes {
        if let EdgeChange::Add { index, item } = change {
            edges.insert(index.unwrap_or(edges.len()).min(edges.len()), item.clone());
        }
    }
}

fn validate_node_change_targets(
    nodes: &[Node],
    changes: &[NodeChange],
) -> Result<(), GraphDataError> {
    let existing: HashSet<_> = nodes.iter().map(|node| node.id).collect();
    for change in changes {
        let id = match change {
            NodeChange::Add { .. } => continue,
            NodeChange::Remove { id }
            | NodeChange::Replace { id, .. }
            | NodeChange::Select { id, .. }
            | NodeChange::Position { id, .. }
            | NodeChange::Dimensions { id, .. } => *id,
        };
        if !existing.contains(&id) {
            return Err(GraphDataError::UnknownNode(id));
        }
    }
    Ok(())
}

fn validate_edge_change_targets(
    edges: &[Edge],
    changes: &[EdgeChange],
) -> Result<(), GraphDataError> {
    let existing: HashSet<_> = edges.iter().map(|edge| edge.id).collect();
    for change in changes {
        let id = match change {
            EdgeChange::Add { .. } => continue,
            EdgeChange::Remove { id }
            | EdgeChange::Replace { id, .. }
            | EdgeChange::Select { id, .. } => *id,
        };
        if !existing.contains(&id) {
            return Err(GraphDataError::UnknownEdge(id));
        }
    }
    Ok(())
}

fn validate_changed_connections(
    edges: &[Edge],
    changes: &[EdgeChange],
) -> Result<(), GraphDataError> {
    let changed: HashSet<_> = changes
        .iter()
        .filter_map(|change| match change {
            EdgeChange::Add { item, .. } | EdgeChange::Replace { item, .. } => Some(item.id),
            _ => None,
        })
        .collect();
    for (index, edge) in edges.iter().enumerate() {
        if changed.contains(&edge.id)
            && edges
                .iter()
                .enumerate()
                .any(|(other_index, other)| other_index != index && edge.connects(other))
        {
            return Err(GraphDataError::DuplicateConnection {
                source: edge.source,
                target: edge.target,
            });
        }
    }
    Ok(())
}

/// Applies a node change batch atomically, rejecting any resulting invalid graph.
pub fn apply_node_changes(
    nodes: &mut Vec<Node>,
    edges: &[Edge],
    changes: &[NodeChange],
) -> Result<(), GraphDataError> {
    validate_node_change_targets(nodes, changes)?;
    for change in changes {
        if let NodeChange::Replace { id, item } = change {
            if *id != item.id {
                return Err(GraphDataError::NodeReplacementIdMismatch {
                    targeted: *id,
                    replacement: item.id,
                });
            }
        }
    }
    let mut candidate = nodes.clone();
    apply_node_changes_unchecked(&mut candidate, changes);
    crate::data::compile_edges(&candidate, edges)?;
    *nodes = candidate;
    Ok(())
}

/// Applies an edge change batch atomically, rejecting any resulting invalid graph.
pub fn apply_edge_changes(
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    changes: &[EdgeChange],
) -> Result<(), GraphDataError> {
    validate_edge_change_targets(edges, changes)?;
    let mut added_ids: HashSet<_> = edges.iter().map(|edge| edge.id).collect();
    for change in changes {
        match change {
            EdgeChange::Add { item, .. } if !added_ids.insert(item.id) => {
                return Err(GraphDataError::DuplicateEdge(item.id));
            }
            EdgeChange::Replace { id, item } if *id != item.id => {
                return Err(GraphDataError::EdgeReplacementIdMismatch {
                    targeted: *id,
                    replacement: item.id,
                });
            }
            _ => {}
        }
    }
    let mut candidate = edges.clone();
    apply_edge_changes_unchecked(&mut candidate, changes);
    crate::data::compile_edges(nodes, &candidate)?;
    validate_changed_connections(&candidate, changes)?;
    *edges = candidate;
    Ok(())
}

pub fn diff_node_changes(old: &[Node], new: &[Node]) -> Vec<NodeChange> {
    let old_lookup: HashMap<_, _> = old.iter().map(|node| (node.id, node)).collect();
    let new_ids: HashSet<_> = new.iter().map(|node| node.id).collect();
    let mut changes = old
        .iter()
        .filter(|node| !new_ids.contains(&node.id))
        .map(|node| NodeChange::Remove { id: node.id })
        .collect::<Vec<_>>();
    for (index, node) in new.iter().enumerate() {
        match old_lookup.get(&node.id) {
            None => changes.push(NodeChange::Add {
                index: Some(index),
                item: node.clone(),
            }),
            Some(old) if *old != node => changes.push(NodeChange::Replace {
                id: node.id,
                item: node.clone(),
            }),
            _ => {}
        }
    }
    changes
}

pub fn diff_edge_changes(old: &[Edge], new: &[Edge]) -> Vec<EdgeChange> {
    let old_lookup: HashMap<_, _> = old.iter().map(|edge| (edge.id, edge)).collect();
    let new_ids: HashSet<_> = new.iter().map(|edge| edge.id).collect();
    let mut changes = old
        .iter()
        .filter(|edge| !new_ids.contains(&edge.id))
        .map(|edge| EdgeChange::Remove { id: edge.id })
        .collect::<Vec<_>>();
    for (index, edge) in new.iter().enumerate() {
        match old_lookup.get(&edge.id) {
            None => changes.push(EdgeChange::Add {
                index: Some(index),
                item: edge.clone(),
            }),
            Some(old) if *old != edge => changes.push(EdgeChange::Replace {
                id: edge.id,
                item: edge.clone(),
            }),
            _ => {}
        }
    }
    changes
}

pub fn constrain_node_position(node: &Node, position: WorldPoint) -> WorldPoint {
    let Some(extent) = node.extent else {
        return position;
    };
    let left = node.size.width * node.origin.x;
    let right = node.size.width * (1.0 - node.origin.x);
    let top = node.size.height * node.origin.y;
    let bottom = node.size.height * (1.0 - node.origin.y);
    fn constrain_axis(value: f32, min: f32, max: f32) -> f32 {
        if !value.is_finite() || !min.is_finite() || !max.is_finite() {
            return value;
        }
        if min <= max {
            value.clamp(min, max)
        } else {
            // The node is larger than its extent. Centering is stable and
            // avoids choosing one arbitrary overflowing side.
            min + (max - min) * 0.5
        }
    }
    WorldPoint::new(
        constrain_axis(
            position.x,
            extent.origin.x + left,
            extent.origin.x + extent.size.width - right,
        ),
        constrain_axis(
            position.y,
            extent.origin.y + top,
            extent.origin.y + extent.size.height - bottom,
        ),
    )
}

pub fn node_center(node: &Node) -> WorldPoint {
    WorldPoint::new(
        node.position.x + node.size.width * (0.5 - node.origin.x),
        node.position.y + node.size.height * (0.5 - node.origin.y),
    )
}

pub fn expand_parent_changes(
    nodes: &[Node],
    runtimes: &HashMap<NodeId, NodeRuntime>,
    child_id: NodeId,
    child_bounds: WorldBounds,
) -> Vec<NodeChange> {
    let Some(child) = nodes
        .iter()
        .find(|node| node.id == child_id && node.expand_parent)
    else {
        return Vec::new();
    };
    let Some(parent_id) = child.parent_id else {
        return Vec::new();
    };
    let Some(parent) = nodes.iter().find(|node| node.id == parent_id) else {
        return Vec::new();
    };
    let Some(runtime) = runtimes.get(&parent_id) else {
        return Vec::new();
    };
    let current = runtime.bounds();
    let left = current.origin.x.min(child_bounds.origin.x);
    let top = current.origin.y.min(child_bounds.origin.y);
    let right = (current.origin.x + current.size.width)
        .max(child_bounds.origin.x + child_bounds.size.width);
    let bottom = (current.origin.y + current.size.height)
        .max(child_bounds.origin.y + child_bounds.size.height);
    let size = WorldSize::new(right - left, bottom - top);
    if size == current.size && left == current.origin.x && top == current.origin.y {
        return Vec::new();
    }
    let delta = WorldPoint::new(current.origin.x - left, current.origin.y - top);
    let parent_position = WorldPoint::new(
        parent.position.x - delta.x + size.width * parent.origin.x
            - current.size.width * parent.origin.x,
        parent.position.y - delta.y + size.height * parent.origin.y
            - current.size.height * parent.origin.y,
    );
    vec![
        NodeChange::Position {
            id: parent_id,
            position: Some(parent_position),
            dragging: None,
        },
        NodeChange::Dimensions {
            id: parent_id,
            size: Some(size),
            resizing: None,
        },
        NodeChange::Position {
            id: child_id,
            position: Some(WorldPoint::new(
                child.position.x + delta.x,
                child.position.y + delta.y,
            )),
            dragging: None,
        },
    ]
}

#[derive(Clone, Debug)]
pub struct ConnectionRef {
    pub edge: EdgeId,
    pub other: NodeId,
    pub handle: Option<Arc<str>>,
}

#[derive(Default)]
pub struct EditorStore {
    pub ownership: GraphOwnership,
    pub node_lookup: HashMap<NodeId, usize>,
    pub runtimes: HashMap<NodeId, NodeRuntime>,
    pub edge_lookup: HashMap<EdgeId, usize>,
    pub connections: HashMap<NodeId, Vec<ConnectionRef>>,
    pub handle_connections: HashMap<HandleKey, Vec<ConnectionRef>>,
    pub dirty: DirtyTracker,
    /// Dense node geometry. Written wherever a runtime is written, read by
    /// every per-frame scan.
    pub columns: NodeColumns,
    pub visibility: VisibilityIndex,
    pub edge_revisions: HashMap<EdgeId, u64>,
    optimistic_node_selection: HashMap<NodeId, bool>,
    optimistic_edge_selection: HashMap<EdgeId, bool>,
    edge_endpoints: HashMap<EdgeId, (NodeId, NodeId)>,
    node_specs: HashMap<NodeId, Node>,
    edge_specs: HashMap<EdgeId, Edge>,
}

impl EditorStore {
    pub fn node_selected(&self, node: &Node) -> bool {
        self.optimistic_node_selection
            .get(&node.id)
            .copied()
            .unwrap_or(node.selected)
    }

    pub fn edge_selected(&self, edge: &Edge) -> bool {
        self.optimistic_edge_selection
            .get(&edge.id)
            .copied()
            .unwrap_or(edge.selected)
    }

    fn apply_selection_overlay(&mut self, nodes: &[NodeChange], edges: &[EdgeChange]) {
        for change in nodes {
            if let NodeChange::Select { id, selected } = change {
                self.optimistic_node_selection.insert(*id, *selected);
            }
        }
        for change in edges {
            if let EdgeChange::Select { id, selected } = change {
                self.optimistic_edge_selection.insert(*id, *selected);
            }
        }
    }

    fn clear_selection_overlay(&mut self) {
        self.optimistic_node_selection.clear();
        self.optimistic_edge_selection.clear();
    }

    pub fn rebuild(&mut self, nodes: &[Node], edges: &[Edge]) {
        let old_node_ids: HashSet<_> = self.node_lookup.keys().copied().collect();
        let node_specs_changed = nodes
            .iter()
            .any(|node| self.node_specs.get(&node.id) != Some(node));
        let old_specs = self.edge_specs.clone();
        let new_node_ids: HashSet<_> = nodes.iter().map(|node| node.id).collect();
        let new_endpoints: HashMap<_, _> = edges
            .iter()
            .map(|edge| (edge.id, (edge.source, edge.target)))
            .collect();
        let membership_changed = old_node_ids != new_node_ids
            || nodes
                .iter()
                .enumerate()
                .any(|(index, node)| self.node_lookup.get(&node.id) != Some(&index))
            || self.edge_endpoints.keys().copied().collect::<HashSet<_>>()
                != new_endpoints.keys().copied().collect()
            || edges
                .iter()
                .enumerate()
                .any(|(index, edge)| self.edge_lookup.get(&edge.id) != Some(&index));
        let adjacency_changed = self.edge_endpoints != new_endpoints;
        if membership_changed || adjacency_changed {
            self.dirty.rebuild(edges);
        }
        if node_specs_changed {
            self.dirty.mark_node_spec();
        }
        self.edge_endpoints = new_endpoints;
        self.runtimes.retain(|id, _| new_node_ids.contains(id));
        self.adopt_nodes(nodes);
        self.edge_lookup.clear();
        self.connections.clear();
        self.handle_connections.clear();
        for (index, edge) in edges.iter().enumerate() {
            let id = edge.id;
            self.edge_lookup.insert(id, index);
            self.connections
                .entry(edge.source)
                .or_default()
                .push(ConnectionRef {
                    edge: id,
                    other: edge.target,
                    handle: None,
                });
            let source_key = HandleKey {
                node: edge.source,
                id: edge.source_handle.as_deref().map(Arc::from),
                kind: HandleKind::Source,
            };
            let target_key = HandleKey {
                node: edge.target,
                id: edge.target_handle.as_deref().map(Arc::from),
                kind: HandleKind::Target,
            };
            self.handle_connections
                .entry(source_key)
                .or_default()
                .push(ConnectionRef {
                    edge: id,
                    other: edge.target,
                    handle: edge.target_handle.as_deref().map(Arc::from),
                });
            self.handle_connections
                .entry(target_key)
                .or_default()
                .push(ConnectionRef {
                    edge: id,
                    other: edge.source,
                    handle: edge.source_handle.as_deref().map(Arc::from),
                });
            self.connections
                .entry(edge.target)
                .or_default()
                .push(ConnectionRef {
                    edge: id,
                    other: edge.source,
                    handle: None,
                });
            if old_specs.get(&id) != Some(edge) {
                self.dirty.mark_edge_spec();
                let revision = self.edge_revisions.entry(id).or_default();
                *revision = revision.wrapping_add(1);
                self.dirty.mark_edge(id);
            }
        }
        self.edge_revisions
            .retain(|id, _| self.edge_lookup.contains_key(id));
        self.node_specs = nodes.iter().map(|node| (node.id, node.clone())).collect();
        self.edge_specs = edges.iter().map(|edge| (edge.id, edge.clone())).collect();
        self.rebuild_columns(nodes);
    }

    /// Rebuilds the dense columns from the adopted runtimes. Called whenever
    /// membership or any node specification changes; a frame that only moves
    /// nodes writes the columns in place instead.
    fn rebuild_columns(&mut self, nodes: &[Node]) {
        self.columns.clear();
        for node in nodes {
            let parent = node
                .parent_id
                .and_then(|id| self.node_lookup.get(&id))
                .map_or(NO_PARENT, |index| *index as u32);
            let (absolute, size) = self.runtimes.get(&node.id).map_or_else(
                || (node.position, node.size),
                |runtime| (runtime.position_absolute, runtime.measured),
            );
            self.columns.push(node, absolute, size, parent);
        }
        self.columns.flat = self
            .columns
            .parent
            .iter()
            .all(|parent| *parent == NO_PARENT);
        self.columns.touch();
    }

    pub fn adopt_nodes(&mut self, nodes: &[Node]) {
        self.node_lookup.clear();
        for (index, node) in nodes.iter().enumerate() {
            self.node_lookup.insert(node.id, index);
            let parent = node
                .parent_id
                .and_then(|id| self.runtimes.get(&id))
                .map(|r| r.position_absolute)
                .unwrap_or(WorldPoint::ZERO);
            let runtime = self
                .runtimes
                .entry(node.id)
                .or_insert_with(|| NodeRuntime::from_node(node));
            let absolute = WorldPoint::new(
                parent.x + node.position.x - node.size.width * node.origin.x,
                parent.y + node.position.y - node.size.height * node.origin.y,
            );
            if runtime.position_absolute != absolute || runtime.measured != node.size {
                runtime.position_absolute = absolute;
                runtime.measured = node.size;
                runtime.revision = runtime.revision.wrapping_add(1);
                self.dirty.mark_node(node.id);
            }
        }
    }

    pub fn measure_node(&mut self, id: NodeId, size: WorldSize, handles: Vec<Handle>) -> bool {
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width < 0.0
            || size.height < 0.0
        {
            return false;
        }
        let Some(runtime) = self.runtimes.get_mut(&id) else {
            return false;
        };
        let changed = runtime.measured != size || runtime.handles != handles;
        runtime.measured = size;
        runtime.handles = handles;
        if changed {
            runtime.revision = runtime.revision.wrapping_add(1);
            self.dirty.mark_node(id);
            if let Some(index) = self.node_lookup.get(&id).copied() {
                self.columns.set_size(index, size);
            }
        }
        changed
    }

    pub fn update_runtime_position(
        &mut self,
        id: NodeId,
        position: WorldPoint,
        dragging: bool,
    ) -> bool {
        let Some(runtime) = self.runtimes.get_mut(&id) else {
            return false;
        };
        if runtime.position_absolute != position || runtime.dragging != dragging {
            runtime.position_absolute = position;
            runtime.dragging = dragging;
            runtime.revision = runtime.revision.wrapping_add(1);
            self.dirty.mark_node(id);
            if let Some(index) = self.node_lookup.get(&id).copied() {
                self.columns.set_position(index, position);
            }
        }
        true
    }
    pub fn update_node_from_spec(&mut self, node: &Node, dragging: bool) -> bool {
        let parent = node
            .parent_id
            .and_then(|id| self.runtimes.get(&id))
            .map(|runtime| runtime.position_absolute)
            .unwrap_or(WorldPoint::ZERO);
        let absolute = WorldPoint::new(
            parent.x + node.position.x - node.size.width * node.origin.x,
            parent.y + node.position.y - node.size.height * node.origin.y,
        );
        self.update_runtime_position(node.id, absolute, dragging)
    }
    /// Bulk position sync for a layout frame, where every node moves at once.
    /// Per-node marking would push every adjacent edge id into the dirty set on
    /// every frame, so this marks the graph dirty once instead.
    pub fn sync_positions_from_specs(&mut self, nodes: &[Node]) -> bool {
        let mut moved = false;
        for (index, node) in nodes.iter().enumerate() {
            // Parents precede their children, so the parent's absolute
            // position is already written this pass and can be read straight
            // out of the columns by index.
            let parent_index = self.columns.parent.get(index).copied().unwrap_or(NO_PARENT);
            let parent = if parent_index == NO_PARENT {
                WorldPoint::ZERO
            } else {
                WorldPoint::new(
                    self.columns.x[parent_index as usize],
                    self.columns.y[parent_index as usize],
                )
            };
            let absolute = WorldPoint::new(
                parent.x + node.position.x - node.size.width * node.origin.x,
                parent.y + node.position.y - node.size.height * node.origin.y,
            );
            let Some(runtime) = self.runtimes.get_mut(&node.id) else {
                continue;
            };
            if runtime.position_absolute == absolute && !runtime.dragging {
                continue;
            }
            runtime.position_absolute = absolute;
            runtime.dragging = false;
            runtime.revision = runtime.revision.wrapping_add(1);
            if index < self.columns.len() {
                self.columns.x[index] = absolute.x;
                self.columns.y[index] = absolute.y;
            }
            moved = true;
        }
        if moved {
            self.columns.touch();
        }
        if moved {
            self.dirty.mark_all();
        }
        moved
    }

    pub fn take_dirty(&mut self) -> DirtySet {
        self.dirty.take()
    }

    pub fn nodes_in_rect(
        &self,
        nodes: &[Node],
        rect: WorldBounds,
        mode: SelectionMode,
    ) -> HashSet<NodeId> {
        nodes
            .iter()
            .filter_map(|node| {
                let runtime = self.runtimes.get(&node.id)?;
                bounds_intersect(rect, runtime.bounds(), mode).then_some(node.id)
            })
            .collect()
    }
}

/// Single-threaded model executor. External ownership queues changes without
/// mutating host data; internal ownership applies the same changes immediately.
#[derive(Default)]
pub struct EditorModel {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub store: EditorStore,
    node_changes: Vec<NodeChange>,
    edge_changes: Vec<EdgeChange>,
    node_middleware: Vec<NodeChangeMiddleware>,
    edge_middleware: Vec<EdgeChangeMiddleware>,
}

impl EditorModel {
    pub fn new(
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        ownership: GraphOwnership,
    ) -> Result<Self, GraphDataError> {
        crate::data::compile_edges(&nodes, &edges)?;
        let mut store = EditorStore {
            ownership,
            ..EditorStore::default()
        };
        store.rebuild(&nodes, &edges);
        Ok(Self {
            nodes,
            edges,
            store,
            node_changes: Vec::new(),
            edge_changes: Vec::new(),
            node_middleware: Vec::new(),
            edge_middleware: Vec::new(),
        })
    }

    pub fn replace_external(
        &mut self,
        nodes: Vec<Node>,
        edges: Vec<Edge>,
    ) -> Result<(), GraphDataError> {
        crate::data::compile_edges(&nodes, &edges)?;
        self.nodes = nodes;
        self.edges = edges;
        self.store.clear_selection_overlay();
        self.store.rebuild(&self.nodes, &self.edges);
        Ok(())
    }

    pub fn emit_nodes(
        &mut self,
        changes: impl IntoIterator<Item = NodeChange>,
    ) -> Result<(), GraphDataError> {
        let mut changes: Vec<_> = changes.into_iter().collect();
        for middleware in &self.node_middleware {
            middleware(&mut changes);
        }
        let mut projected = self.nodes.clone();
        apply_node_changes(&mut projected, &self.edges, &changes)?;
        self.store.apply_selection_stacking(&changes);
        if self.store.ownership == GraphOwnership::External {
            self.store.apply_selection_overlay(&changes, &[]);
            if changes
                .iter()
                .any(|change| matches!(change, NodeChange::Select { .. }))
            {
                self.store.dirty.mark_selection();
            }
        } else {
            self.nodes = projected;
            if changes.iter().any(|change| {
                matches!(
                    change,
                    NodeChange::Add { .. } | NodeChange::Remove { .. } | NodeChange::Replace { .. }
                )
            }) {
                self.store.rebuild(&self.nodes, &self.edges);
            } else {
                for change in &changes {
                    match change {
                        NodeChange::Position { id, dragging, .. } => {
                            if let Some(index) = self.store.node_lookup.get(id).copied() {
                                let node = self.nodes[index].clone();
                                self.store
                                    .update_node_from_spec(&node, dragging.unwrap_or(false));
                            }
                        }
                        NodeChange::Dimensions { id, .. } => {
                            self.store.dirty.mark_node_spec();
                            if let Some(index) = self.store.node_lookup.get(id).copied() {
                                let node = self.nodes[index].clone();
                                let handles = self
                                    .store
                                    .runtimes
                                    .get(id)
                                    .map(|runtime| runtime.handles.clone())
                                    .unwrap_or_default();
                                self.store.measure_node(*id, node.size, handles);
                                self.store.update_node_from_spec(&node, false);
                            }
                        }
                        NodeChange::Select { .. } => self.store.dirty.mark_selection(),
                        _ => {}
                    }
                }
            }
        }
        self.node_changes.extend(changes);
        Ok(())
    }

    pub fn emit_edges(
        &mut self,
        changes: impl IntoIterator<Item = EdgeChange>,
    ) -> Result<(), GraphDataError> {
        let mut changes: Vec<_> = changes.into_iter().collect();
        for middleware in &self.edge_middleware {
            middleware(&mut changes);
        }
        let mut projected = self.edges.clone();
        apply_edge_changes(&self.nodes, &mut projected, &changes)?;
        if self.store.ownership == GraphOwnership::External {
            self.store.apply_selection_overlay(&[], &changes);
            if changes
                .iter()
                .any(|change| matches!(change, EdgeChange::Select { .. }))
            {
                self.store.dirty.mark_selection();
            }
        } else {
            self.edges = projected;
            if changes
                .iter()
                .any(|change| !matches!(change, EdgeChange::Select { .. }))
            {
                self.store.rebuild(&self.nodes, &self.edges);
            } else if !changes.is_empty() {
                self.store.dirty.mark_selection();
            }
        }
        self.edge_changes.extend(changes);
        Ok(())
    }

    /// Validates and commits node and edge changes against one projected graph.
    /// This is required for structural operations, such as deleting a node and
    /// its incident edges, where neither half is independently valid.
    pub fn emit_graph_changes(
        &mut self,
        node_changes: impl IntoIterator<Item = NodeChange>,
        edge_changes: impl IntoIterator<Item = EdgeChange>,
    ) -> Result<(), GraphDataError> {
        let mut node_changes: Vec<_> = node_changes.into_iter().collect();
        let mut edge_changes: Vec<_> = edge_changes.into_iter().collect();
        for middleware in &self.node_middleware {
            middleware(&mut node_changes);
        }
        for middleware in &self.edge_middleware {
            middleware(&mut edge_changes);
        }
        validate_node_change_targets(&self.nodes, &node_changes)?;
        validate_edge_change_targets(&self.edges, &edge_changes)?;

        for change in &node_changes {
            if let NodeChange::Replace { id, item } = change {
                if *id != item.id {
                    return Err(GraphDataError::NodeReplacementIdMismatch {
                        targeted: *id,
                        replacement: item.id,
                    });
                }
            }
        }
        let mut added_edge_ids: HashSet<_> = self.edges.iter().map(|edge| edge.id).collect();
        for change in &edge_changes {
            match change {
                EdgeChange::Add { item, .. } if !added_edge_ids.insert(item.id) => {
                    return Err(GraphDataError::DuplicateEdge(item.id));
                }
                EdgeChange::Replace { id, item } if *id != item.id => {
                    return Err(GraphDataError::EdgeReplacementIdMismatch {
                        targeted: *id,
                        replacement: item.id,
                    });
                }
                _ => {}
            }
        }

        let mut projected_nodes = self.nodes.clone();
        let mut projected_edges = self.edges.clone();
        apply_node_changes_unchecked(&mut projected_nodes, &node_changes);
        apply_edge_changes_unchecked(&mut projected_edges, &edge_changes);
        crate::data::compile_edges(&projected_nodes, &projected_edges)?;
        validate_changed_connections(&projected_edges, &edge_changes)?;

        self.store.apply_selection_stacking(&node_changes);
        if self.store.ownership == GraphOwnership::External {
            self.store
                .apply_selection_overlay(&node_changes, &edge_changes);
            if node_changes
                .iter()
                .any(|change| matches!(change, NodeChange::Select { .. }))
                || edge_changes
                    .iter()
                    .any(|change| matches!(change, EdgeChange::Select { .. }))
            {
                self.store.dirty.mark_selection();
            }
        } else {
            self.nodes = projected_nodes;
            self.edges = projected_edges;
            self.store.rebuild(&self.nodes, &self.edges);
        }
        self.node_changes.extend(node_changes);
        self.edge_changes.extend(edge_changes);
        Ok(())
    }

    pub fn take_changes(&mut self) -> (Vec<NodeChange>, Vec<EdgeChange>) {
        (
            std::mem::take(&mut self.node_changes),
            std::mem::take(&mut self.edge_changes),
        )
    }
    pub fn flush_to_host(&mut self, host: &mut impl GraphHost) {
        let (nodes, edges) = self.take_changes();
        if !nodes.is_empty() {
            host.emit_node_changes(nodes)
        }
        if !edges.is_empty() {
            host.emit_edge_changes(edges)
        }
    }
    pub fn add_connection_with_host(
        &mut self,
        connection: &Connection,
        edge: Edge,
        host: &impl GraphHost,
    ) -> bool {
        host.validate_connection(connection) && self.add_edge(edge)
    }
    pub fn delete_selected_with_host(&mut self, host: &mut impl GraphHost) -> bool {
        let nodes = self
            .nodes
            .iter()
            .filter(|node| self.store.node_selected(node) && node.deletable)
            .map(|node| node.id)
            .collect::<HashSet<_>>();
        let mut edges = self
            .edges
            .iter()
            .filter(|edge| self.store.edge_selected(edge) && edge.deletable)
            .map(|edge| edge.id)
            .collect::<HashSet<_>>();
        for edge in &self.edges {
            if edge.deletable && (nodes.contains(&edge.source) || nodes.contains(&edge.target)) {
                edges.insert(edge.id);
            }
        }
        match host.before_delete(DeleteSet { nodes, edges }) {
            DeleteDecision::Reject => false,
            DeleteDecision::Accept(set) => self
                .emit_graph_changes(
                    set.nodes.into_iter().map(|id| NodeChange::Remove { id }),
                    set.edges.into_iter().map(|id| EdgeChange::Remove { id }),
                )
                .is_ok(),
        }
    }
    pub fn add_node_middleware(
        &mut self,
        middleware: impl Fn(&mut Vec<NodeChange>) + Send + Sync + 'static,
    ) {
        self.node_middleware.push(Arc::new(middleware));
    }
    pub fn add_edge_middleware(
        &mut self,
        middleware: impl Fn(&mut Vec<EdgeChange>) + Send + Sync + 'static,
    ) {
        self.edge_middleware.push(Arc::new(middleware));
    }
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.store
            .node_lookup
            .get(&id)
            .and_then(|index| self.nodes.get(*index))
    }
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.store
            .edge_lookup
            .get(&id)
            .and_then(|index| self.edges.get(*index))
    }
    pub fn connected_edges(&self, id: NodeId) -> impl Iterator<Item = &Edge> {
        self.store
            .connections
            .get(&id)
            .into_iter()
            .flatten()
            .filter_map(|connection| self.edge(connection.edge))
    }
    pub fn intersecting_nodes(&self, bounds: WorldBounds, mode: SelectionMode) -> HashSet<NodeId> {
        self.store.nodes_in_rect(&self.nodes, bounds, mode)
    }
    pub fn update_node_internals(
        &mut self,
        id: NodeId,
        size: WorldSize,
        handles: Vec<Handle>,
    ) -> bool {
        self.store.measure_node(id, size, handles)
    }

    pub fn select_node(&mut self, id: NodeId, multi: bool, toggle: bool) -> bool {
        let changes = self
            .nodes
            .iter()
            .filter_map(|node| {
                let selected = if node.id == id {
                    if toggle && multi {
                        !self.store.node_selected(node)
                    } else {
                        true
                    }
                } else if multi {
                    self.store.node_selected(node)
                } else {
                    false
                };
                (self.store.node_selected(node) != selected).then_some(NodeChange::Select {
                    id: node.id,
                    selected,
                })
            })
            .collect::<Vec<_>>();
        let edge_changes = if multi {
            Vec::new()
        } else {
            self.edges
                .iter()
                .filter(|edge| self.store.edge_selected(edge))
                .map(|edge| EdgeChange::Select {
                    id: edge.id,
                    selected: false,
                })
                .collect()
        };
        self.emit_graph_changes(changes, edge_changes).is_ok()
    }

    pub(crate) fn select_node_for_pointer(&mut self, id: NodeId, shift: bool) -> bool {
        let already_selected = self
            .node(id)
            .is_some_and(|node| self.store.node_selected(node));
        if shift || !already_selected {
            self.select_node(id, shift, shift)
        } else {
            true
        }
    }

    pub fn select_edge(&mut self, id: EdgeId, multi: bool, toggle: bool) -> bool {
        let node_changes = if multi {
            Vec::new()
        } else {
            self.nodes
                .iter()
                .filter(|node| self.store.node_selected(node))
                .map(|node| NodeChange::Select {
                    id: node.id,
                    selected: false,
                })
                .collect()
        };
        let edge_changes = self
            .edges
            .iter()
            .filter_map(|edge| {
                let selected = if edge.id == id {
                    if toggle && multi {
                        !self.store.edge_selected(edge)
                    } else {
                        true
                    }
                } else if multi {
                    self.store.edge_selected(edge)
                } else {
                    false
                };
                (self.store.edge_selected(edge) != selected).then_some(EdgeChange::Select {
                    id: edge.id,
                    selected,
                })
            })
            .collect::<Vec<_>>();
        self.emit_graph_changes(node_changes, edge_changes).is_ok()
    }

    pub fn clear_selection(&mut self) -> bool {
        let nodes = self
            .nodes
            .iter()
            .filter(|node| self.store.node_selected(node))
            .map(|node| NodeChange::Select {
                id: node.id,
                selected: false,
            })
            .collect::<Vec<_>>();
        let edges = self
            .edges
            .iter()
            .filter(|edge| self.store.edge_selected(edge))
            .map(|edge| EdgeChange::Select {
                id: edge.id,
                selected: false,
            })
            .collect::<Vec<_>>();
        self.emit_graph_changes(nodes, edges).is_ok()
    }

    pub fn select_all(&mut self) -> bool {
        let nodes = self
            .nodes
            .iter()
            .filter(|node| node.selectable && !self.store.node_selected(node))
            .map(|node| NodeChange::Select {
                id: node.id,
                selected: true,
            })
            .collect::<Vec<_>>();
        let edges = self
            .edges
            .iter()
            .filter(|edge| edge.selectable && !self.store.edge_selected(edge))
            .map(|edge| EdgeChange::Select {
                id: edge.id,
                selected: true,
            })
            .collect::<Vec<_>>();
        self.emit_graph_changes(nodes, edges).is_ok()
    }

    /// Shared movement operation for keyboard and pointer controllers.
    pub fn move_selected(
        &mut self,
        delta: WorldPoint,
        snap: Option<WorldSize>,
        dragging: bool,
    ) -> bool {
        let targets = self
            .nodes
            .iter()
            .filter(|node| self.store.node_selected(node) && node.draggable)
            .map(|node| {
                let current = self.store.node_position_absolute(node);
                let mut position = WorldPoint::new(current.x + delta.x, current.y + delta.y);
                if let Some(grid) = snap {
                    if grid.width > 0.0 {
                        position.x = (position.x / grid.width).round() * grid.width;
                    }
                    if grid.height > 0.0 {
                        position.y = (position.y / grid.height).round() * grid.height;
                    }
                }
                (node.id, position)
            })
            .collect::<Vec<_>>();
        self.move_nodes(&targets, dragging)
    }

    pub fn move_nodes(&mut self, targets: &[(NodeId, WorldPoint)], dragging: bool) -> bool {
        let mut changes = targets
            .iter()
            .filter_map(|(id, position)| {
                let node = self.node(*id)?;
                let relative = self.store.position_relative_to_parent(node, *position);
                Some(NodeChange::Position {
                    id: *id,
                    position: Some(constrain_node_position(node, relative)),
                    dragging: Some(dragging),
                })
            })
            .collect::<Vec<_>>();
        if !targets.iter().any(|(id, _)| {
            self.node(*id)
                .is_some_and(|node| node.expand_parent && node.parent_id.is_some())
        }) {
            return self.emit_nodes(changes).is_ok();
        }
        let mut projected = self.nodes.clone();
        apply_node_changes_unchecked(&mut projected, &changes);
        for (id, _) in targets {
            let Some(before) = self.node(*id) else {
                continue;
            };
            let Some(after) = projected.iter().find(|node| node.id == *id) else {
                continue;
            };
            let Some(runtime) = self.store.runtimes.get(id) else {
                continue;
            };
            let delta = WorldPoint::new(
                after.position.x - before.position.x,
                after.position.y - before.position.y,
            );
            let bounds = runtime.bounds();
            let expanded = expand_parent_changes(
                &projected,
                &self.store.runtimes,
                *id,
                WorldBounds::new(
                    WorldPoint::new(bounds.origin.x + delta.x, bounds.origin.y + delta.y),
                    bounds.size,
                ),
            );
            apply_node_changes_unchecked(&mut projected, &expanded);
            changes.extend(expanded);
        }
        self.emit_nodes(changes).is_ok()
    }

    pub fn resize_node(
        &mut self,
        id: NodeId,
        requested: WorldSize,
        min: WorldSize,
        max: WorldSize,
        resizing: bool,
    ) -> bool {
        if !self.store.node_lookup.contains_key(&id) {
            return false;
        }
        if !requested.width.is_finite()
            || !requested.height.is_finite()
            || !min.width.is_finite()
            || !min.height.is_finite()
            || !max.width.is_finite()
            || !max.height.is_finite()
            || min.width > max.width
            || min.height > max.height
        {
            return false;
        }
        let size = WorldSize::new(
            requested.width.clamp(min.width, max.width),
            requested.height.clamp(min.height, max.height),
        );
        self.emit_nodes([NodeChange::Dimensions {
            id,
            size: Some(size),
            resizing: Some(resizing),
        }])
        .is_ok()
    }

    pub fn resize_node_from_bounds(
        &mut self,
        id: NodeId,
        bounds: WorldBounds,
        resizing: bool,
    ) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        let absolute = WorldPoint::new(
            bounds.origin.x + bounds.size.width * node.origin.x,
            bounds.origin.y + bounds.size.height * node.origin.y,
        );
        let position = self.store.position_relative_to_parent(node, absolute);
        let mut changes = vec![
            NodeChange::Position {
                id,
                position: Some(position),
                dragging: None,
            },
            NodeChange::Dimensions {
                id,
                size: Some(bounds.size),
                resizing: Some(resizing),
            },
        ];
        let expanded = expand_parent_changes(&self.nodes, &self.store.runtimes, id, bounds);
        changes.extend(expanded);
        self.emit_nodes(changes).is_ok()
    }

    pub fn select_rect(&mut self, rect: WorldBounds, mode: SelectionMode, additive: bool) -> bool {
        let inside = self.store.nodes_in_rect(&self.nodes, rect, mode);
        let selected_nodes: HashSet<_> = self
            .nodes
            .iter()
            .filter(|node| {
                inside.contains(&node.id) || (additive && self.store.node_selected(node))
            })
            .map(|node| node.id)
            .collect();
        let node_changes = self
            .nodes
            .iter()
            .filter_map(|node| {
                let selected = selected_nodes.contains(&node.id);
                (selected != self.store.node_selected(node)).then_some(NodeChange::Select {
                    id: node.id,
                    selected,
                })
            })
            .collect::<Vec<_>>();
        let edge_changes = self
            .edges
            .iter()
            .filter_map(|edge| {
                let selected = edge.selectable
                    && selected_nodes.contains(&edge.source)
                    && selected_nodes.contains(&edge.target);
                (selected != self.store.edge_selected(edge)).then_some(EdgeChange::Select {
                    id: edge.id,
                    selected,
                })
            })
            .collect::<Vec<_>>();
        self.emit_graph_changes(node_changes, edge_changes).is_ok()
    }

    pub fn delete_selected(
        &mut self,
        policy: impl FnOnce(&mut HashSet<NodeId>, &mut HashSet<EdgeId>) -> bool,
    ) -> bool {
        let mut nodes: HashSet<_> = self
            .nodes
            .iter()
            .filter(|node| self.store.node_selected(node) && node.deletable)
            .map(|node| node.id)
            .collect();
        let mut edges: HashSet<_> = self
            .edges
            .iter()
            .filter(|edge| self.store.edge_selected(edge) && edge.deletable)
            .map(|edge| edge.id)
            .collect();
        for edge in &self.edges {
            if edge.deletable && (nodes.contains(&edge.source) || nodes.contains(&edge.target)) {
                edges.insert(edge.id);
            }
        }
        if !policy(&mut nodes, &mut edges) {
            return false;
        }
        self.emit_graph_changes(
            nodes.into_iter().map(|id| NodeChange::Remove { id }),
            edges.into_iter().map(|id| EdgeChange::Remove { id }),
        )
        .is_ok()
    }

    pub fn add_edge_with_id(&mut self, edge: Edge) -> bool {
        if self.edges.iter().any(|existing| existing.connects(&edge)) {
            return false;
        }
        self.emit_edges([EdgeChange::Add {
            index: None,
            item: edge,
        }])
        .is_ok()
    }

    pub fn add_edge(&mut self, edge: Edge) -> bool {
        self.add_edge_with_id(edge)
    }

    pub fn reconnect(
        &mut self,
        id: EdgeId,
        intent: ConnectionIntent,
        connection: &Connection,
    ) -> bool {
        let Some(mut edge) = self.edges.iter().find(|edge| edge.id == id).cloned() else {
            return false;
        };
        match intent {
            ConnectionIntent::ReconnectSource(_) => {
                edge.source = connection.source.node;
                edge.source_handle = connection.source.id.as_deref().map(str::to_owned);
            }
            ConnectionIntent::ReconnectTarget(_) => {
                edge.target = connection.target.node;
                edge.target_handle = connection.target.id.as_deref().map(str::to_owned);
            }
            ConnectionIntent::Create => return false,
        }
        self.emit_edges([EdgeChange::Replace { id, item: edge }])
            .is_ok()
    }
}

/// Tests two world-space rectangles using containment or overlap semantics.
pub fn bounds_intersect(a: WorldBounds, b: WorldBounds, mode: SelectionMode) -> bool {
    let a_right = a.origin.x + a.size.width;
    let a_bottom = a.origin.y + a.size.height;
    let b_right = b.origin.x + b.size.width;
    let b_bottom = b.origin.y + b.size.height;
    match mode {
        SelectionMode::Full => {
            b.origin.x >= a.origin.x
                && b.origin.y >= a.origin.y
                && b_right <= a_right
                && b_bottom <= a_bottom
        }
        SelectionMode::Partial => {
            b_right >= a.origin.x
                && b_bottom >= a.origin.y
                && b.origin.x <= a_right
                && b.origin.y <= a_bottom
        }
    }
}

pub trait NodeRenderer: Send + Sync {
    fn render_type(&self) -> &str;
}
pub trait EdgeRenderer: Send + Sync {
    fn render_type(&self) -> &str;
}

#[derive(Default)]
pub struct RendererRegistry {
    nodes: HashMap<Arc<str>, Arc<dyn NodeRenderer>>,
    edges: HashMap<Arc<str>, Arc<dyn EdgeRenderer>>,
}

impl RendererRegistry {
    pub fn register_node(&mut self, kind: impl Into<Arc<str>>, renderer: Arc<dyn NodeRenderer>) {
        self.nodes.insert(kind.into(), renderer);
    }
    pub fn register_edge(&mut self, kind: impl Into<Arc<str>>, renderer: Arc<dyn EdgeRenderer>) {
        self.edges.insert(kind.into(), renderer);
    }
    pub fn node(&self, kind: &str) -> Option<&Arc<dyn NodeRenderer>> {
        self.nodes.get(kind).or_else(|| self.nodes.get("default"))
    }
    pub fn edge(&self, kind: &str) -> Option<&Arc<dyn EdgeRenderer>> {
        self.edges.get(kind).or_else(|| self.edges.get("default"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dense columns are what every per-frame scan reads, so they have to
    /// agree with the runtime map they mirror. A layout frame writes them
    /// through parent *indices* rather than a hash map keyed by parent id;
    /// this checks that the child still lands where the hierarchy puts it.
    #[test]
    fn dense_columns_mirror_the_runtimes_through_parent_indices() {
        let parent =
            Node::new(1u64, WorldPoint::new(100.0, 50.0)).with_size(WorldSize::new(40.0, 20.0));
        let mut child =
            Node::new(2u64, WorldPoint::new(5.0, 7.0)).with_size(WorldSize::new(10.0, 10.0));
        child.parent_id = Some(NodeId(1));
        let mut model =
            EditorModel::new(vec![parent, child], Vec::new(), GraphOwnership::Internal).unwrap();

        assert!(
            !model.store.columns.is_flat(),
            "a parented graph is not flat"
        );
        assert_eq!(model.store.columns.parent[0], NO_PARENT);
        assert_eq!(model.store.columns.parent[1], 0);

        // Move the parent the way a layout frame does, then re-derive.
        model.nodes[0].position = WorldPoint::new(300.0, 200.0);
        assert!(model.store.sync_positions_from_specs(&model.nodes));

        for (index, node) in model.nodes.iter().enumerate() {
            let runtime = &model.store.runtimes[&node.id];
            assert_eq!(
                (
                    model.store.columns.x[index],
                    model.store.columns.y[index],
                    model.store.columns.size(index)
                ),
                (
                    runtime.position_absolute.x,
                    runtime.position_absolute.y,
                    runtime.measured
                ),
                "column {index} drifted from its runtime"
            );
        }

        // Parent origin (300,200) with a centered origin puts its top-left at
        // (280,190); the child sits at that plus its own (5,7) less half its
        // own size.
        assert_eq!(model.store.columns.x[0], 280.0);
        assert_eq!(model.store.columns.x[1], 280.0);
        assert_eq!(model.store.columns.y[1], 192.0);
    }

    #[test]
    fn a_graph_without_parents_reports_itself_flat() {
        let model = EditorModel::new(
            vec![
                Node::new(1u64, WorldPoint::ZERO),
                Node::new(2u64, WorldPoint::new(10.0, 10.0)),
            ],
            Vec::new(),
            GraphOwnership::Internal,
        )
        .unwrap();
        assert!(model.store.columns.is_flat());
        assert_eq!(model.store.columns.len(), 2);
    }

    #[test]
    fn external_deletion_queues_nodes_and_incident_edges_atomically() {
        let mut first = Node::new(1u64, WorldPoint::ZERO);
        first.selected = true;
        let second = Node::new(2u64, WorldPoint::ZERO);
        let edge = Edge::new(1u64, 2u64).with_id(7u64);
        let mut model =
            EditorModel::new(vec![first, second], vec![edge], GraphOwnership::External).unwrap();

        assert!(model.delete_selected(|_, _| true));
        let (nodes, edges) = model.take_changes();
        assert_eq!(nodes, vec![NodeChange::Remove { id: NodeId(1) }]);
        assert_eq!(edges, vec![EdgeChange::Remove { id: EdgeId(7) }]);
    }

    #[test]
    fn rejected_edge_addition_reports_failure_and_queues_nothing() {
        let mut model = EditorModel::new(
            vec![Node::new(1u64, WorldPoint::ZERO)],
            vec![],
            GraphOwnership::External,
        )
        .unwrap();

        assert!(!model.add_edge_with_id(Edge::new(1u64, 99u64).with_id(3u64)));
        assert_eq!(model.take_changes(), (vec![], vec![]));
    }

    #[test]
    fn malformed_resize_limits_and_small_extents_do_not_panic() {
        let mut node = Node::new(1u64, WorldPoint::ZERO);
        node.size = WorldSize::new(100.0, 100.0);
        node.extent = Some(WorldBounds::new(
            WorldPoint::ZERO,
            WorldSize::new(10.0, 10.0),
        ));
        let constrained = constrain_node_position(&node, WorldPoint::new(5.0, 5.0));
        assert!(constrained.x.is_finite() && constrained.y.is_finite());

        let mut model = EditorModel::new(vec![node], vec![], GraphOwnership::Internal).unwrap();
        assert!(!model.resize_node(
            NodeId(1),
            WorldSize::new(5.0, 5.0),
            WorldSize::new(10.0, 0.0),
            WorldSize::new(1.0, 20.0),
            false,
        ));
        assert!(!model.resize_node(
            NodeId(1),
            WorldSize::new(5.0, 5.0),
            WorldSize::new(f32::NAN, 0.0),
            WorldSize::new(10.0, 20.0),
            false,
        ));
    }

    #[test]
    fn direct_changes_reject_unknown_targets_and_duplicate_connections() {
        let nodes = vec![
            Node::new(1u64, WorldPoint::ZERO),
            Node::new(2u64, WorldPoint::ZERO),
        ];
        let existing = Edge::new(1u64, 2u64).with_id(1u64);
        let mut model = EditorModel::new(nodes, vec![existing], GraphOwnership::External).unwrap();

        assert_eq!(
            model.emit_nodes([NodeChange::Select {
                id: NodeId(99),
                selected: true,
            }]),
            Err(GraphDataError::UnknownNode(NodeId(99)))
        );
        assert_eq!(
            model.emit_edges([EdgeChange::Select {
                id: EdgeId(99),
                selected: true,
            }]),
            Err(GraphDataError::UnknownEdge(EdgeId(99)))
        );
        assert!(matches!(
            model.emit_edges([EdgeChange::Add {
                index: None,
                item: Edge::new(1u64, 2u64).with_id(2u64),
            }]),
            Err(GraphDataError::DuplicateConnection { .. })
        ));
        assert_eq!(model.take_changes(), (vec![], vec![]));
    }

    #[test]
    fn most_recently_selected_node_is_raised_without_a_drag() {
        let nodes = vec![
            Node::new(1u64, WorldPoint::ZERO),
            Node::new(2u64, WorldPoint::ZERO),
        ];
        let mut model = EditorModel::new(nodes, vec![], GraphOwnership::Internal).unwrap();

        assert!(model.select_node(NodeId(1), false, false));
        assert!(model.select_node(NodeId(2), false, false));

        assert!(model.store.runtimes[&NodeId(2)].z > model.store.runtimes[&NodeId(1)].z);
        assert!(!model.store.runtimes[&NodeId(2)].dragging);
        assert!(model.clear_selection());
        assert_eq!(model.store.runtimes[&NodeId(2)].z, 0);
    }

    #[test]
    fn external_position_change_is_validated_and_queued_without_mutating_snapshot() {
        let original = WorldPoint::new(1.0, 2.0);
        let mut model = EditorModel::new(
            vec![Node::new(1u64, original)],
            vec![],
            GraphOwnership::External,
        )
        .unwrap();
        let changed = WorldPoint::new(3.0, 4.0);

        model
            .emit_nodes([NodeChange::Position {
                id: NodeId(1),
                position: Some(changed),
                dragging: Some(false),
            }])
            .unwrap();

        assert_eq!(model.nodes[0].position, original);
        assert_eq!(
            model.take_changes().0,
            vec![NodeChange::Position {
                id: NodeId(1),
                position: Some(changed),
                dragging: Some(false),
            }]
        );
    }

    #[test]
    fn non_finite_position_change_is_rejected_and_not_queued() {
        let original = WorldPoint::new(1.0, 2.0);
        let mut model = EditorModel::new(
            vec![Node::new(1u64, original)],
            vec![],
            GraphOwnership::Internal,
        )
        .unwrap();

        assert_eq!(
            model.emit_nodes([NodeChange::Position {
                id: NodeId(1),
                position: Some(WorldPoint::new(f32::NAN, 4.0)),
                dragging: Some(false),
            }]),
            Err(GraphDataError::NonFiniteNodeGeometry(NodeId(1)))
        );
        assert_eq!(model.nodes[0].position, original);
        assert_eq!(model.take_changes(), (vec![], vec![]));
    }

    #[test]
    fn changes_apply_in_order() {
        let mut nodes = vec![Node::new(1u64, WorldPoint::ZERO)];
        apply_node_changes(
            &mut nodes,
            &[],
            &[
                NodeChange::Position {
                    id: NodeId(1),
                    position: Some(WorldPoint::new(2.0, 3.0)),
                    dragging: Some(true),
                },
                NodeChange::Select {
                    id: NodeId(1),
                    selected: true,
                },
            ],
        )
        .unwrap();
        assert_eq!(nodes[0].position, WorldPoint::new(2.0, 3.0));
        assert!(nodes[0].selected);
    }

    #[test]
    fn model_and_change_apis_reject_invalid_graph_states_atomically() {
        let node = Node::new(1u64, WorldPoint::ZERO);
        assert!(matches!(
            EditorModel::new(
                vec![node.clone(), node.clone()],
                vec![],
                GraphOwnership::Internal
            ),
            Err(GraphDataError::DuplicateNode(NodeId(1)))
        ));

        let mut nodes = vec![node.clone()];
        let original_nodes = nodes.clone();
        assert_eq!(
            apply_node_changes(
                &mut nodes,
                &[],
                &[NodeChange::Replace {
                    id: NodeId(1),
                    item: Node::new(2u64, WorldPoint::ZERO),
                }]
            ),
            Err(GraphDataError::NodeReplacementIdMismatch {
                targeted: NodeId(1),
                replacement: NodeId(2),
            })
        );
        assert_eq!(nodes, original_nodes);

        let mut edges = vec![Edge::new(1u64, 1u64).with_id(7u64)];
        let original_edges = edges.clone();
        assert_eq!(
            apply_edge_changes(
                &nodes,
                &mut edges,
                &[EdgeChange::Add {
                    index: None,
                    item: Edge::new(1u64, 1u64).with_id(7u64),
                }]
            ),
            Err(GraphDataError::DuplicateEdge(EdgeId(7)))
        );
        assert_eq!(edges, original_edges);
    }

    #[test]
    fn emit_and_replace_reject_unknown_endpoints_and_parent_order() {
        let mut model = EditorModel::new(
            vec![Node::new(1u64, WorldPoint::ZERO)],
            vec![],
            GraphOwnership::Internal,
        )
        .unwrap();
        assert_eq!(
            model.emit_edges([EdgeChange::Add {
                index: None,
                item: Edge::new(1u64, 2u64).with_id(9u64),
            }]),
            Err(GraphDataError::UnknownEndpoint(NodeId(2)))
        );
        assert!(model.edges.is_empty());

        assert_eq!(
            model.replace_external(
                vec![
                    Node::new(2u64, WorldPoint::ZERO).with_parent(1u64),
                    Node::new(1u64, WorldPoint::ZERO),
                ],
                vec![],
            ),
            Err(GraphDataError::ParentAfterChild {
                parent: NodeId(1),
                child: NodeId(2),
            })
        );
        assert_eq!(model.nodes, vec![Node::new(1u64, WorldPoint::ZERO)]);
    }

    #[test]
    fn spatial_selection_supports_full_and_partial() {
        let nodes =
            vec![Node::new(1u64, WorldPoint::new(5.0, 5.0)).with_size(WorldSize::new(10.0, 10.0))];
        let mut store = EditorStore::default();
        store.rebuild(&nodes, &[]);
        let rect = WorldBounds::new(WorldPoint::ZERO, WorldSize::new(10.0, 10.0));
        assert!(store
            .nodes_in_rect(&nodes, rect, SelectionMode::Partial)
            .contains(&NodeId(1)));
        assert!(store
            .nodes_in_rect(&nodes, rect, SelectionMode::Full)
            .contains(&NodeId(1)));
    }

    #[test]
    fn child_positions_are_adopted_relative_to_parent() {
        let nodes = vec![
            Node::new(1u64, WorldPoint::new(10.0, 20.0)),
            Node::new(2u64, WorldPoint::new(3.0, 4.0)).with_parent(1u64),
        ];
        let mut store = EditorStore::default();
        store.adopt_nodes(&nodes);
        assert_eq!(
            store.runtimes[&NodeId(2)].position_absolute,
            WorldPoint::new(9.0, 20.0)
        );
        assert_eq!(
            store.node_position_absolute(&nodes[1]),
            WorldPoint::new(11.0, 22.0)
        );
        assert_eq!(
            store.node_center_absolute(&nodes[1]),
            WorldPoint::new(11.0, 22.0)
        );
        assert_eq!(
            store.position_relative_to_parent(&nodes[1], WorldPoint::new(20.0, 30.0)),
            WorldPoint::new(12.0, 12.0)
        );
    }

    #[test]
    fn external_ownership_emits_without_mutating() {
        let mut model = EditorModel::new(
            vec![Node::new(1u64, WorldPoint::ZERO)],
            vec![],
            GraphOwnership::External,
        )
        .unwrap();
        model.select_node(NodeId(1), false, false);
        assert!(!model.nodes[0].selected);
        assert!(model.store.node_selected(&model.nodes[0]));
        assert_eq!(model.take_changes().0.len(), 1);
    }

    #[test]
    fn external_selection_overlay_controls_drag_membership_until_replacement() {
        let mut selected = Node::new(1u64, WorldPoint::ZERO);
        selected.selected = true;
        let mut model = EditorModel::new(
            vec![selected, Node::new(2u64, WorldPoint::ZERO)],
            vec![],
            GraphOwnership::External,
        )
        .unwrap();

        model.select_node(NodeId(2), false, false);
        assert!(!model.store.node_selected(&model.nodes[0]));
        assert!(model.store.node_selected(&model.nodes[1]));
        model.move_selected(WorldPoint::new(5.0, 0.0), None, true);
        let (changes, _) = model.take_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            NodeChange::Position { id, .. } if *id == NodeId(2)
        )));
        assert!(!changes.iter().any(|change| matches!(
            change,
            NodeChange::Position { id, .. } if *id == NodeId(1)
        )));

        let nodes = model.nodes.clone();
        model.replace_external(nodes, vec![]).unwrap();
        assert!(model.store.node_selected(&model.nodes[0]));
        assert!(!model.store.node_selected(&model.nodes[1]));
    }

    #[test]
    fn pointer_down_on_selected_node_preserves_group_for_drag() {
        let mut first = Node::new(1u64, WorldPoint::ZERO);
        first.selected = true;
        let mut second = Node::new(2u64, WorldPoint::new(20.0, 0.0));
        second.selected = true;
        let mut model =
            EditorModel::new(vec![first, second], vec![], GraphOwnership::Internal).unwrap();

        assert!(model.select_node_for_pointer(NodeId(1), false));
        assert!(model.store.node_selected(&model.nodes[0]));
        assert!(model.store.node_selected(&model.nodes[1]));
    }

    #[test]
    fn deleting_node_also_deletes_connected_edges() {
        let mut node = Node::new(1u64, WorldPoint::ZERO);
        node.selected = true;
        let mut model = EditorModel::new(
            vec![node, Node::new(2u64, WorldPoint::ZERO)],
            vec![Edge::new(1u64, 2u64)],
            GraphOwnership::Internal,
        )
        .unwrap();
        assert!(model.delete_selected(|_, _| true));
        assert_eq!(model.nodes.len(), 1);
        assert!(model.edges.is_empty());
    }

    #[test]
    fn keyboard_move_uses_normal_change_path() {
        let mut node = Node::new(1u64, WorldPoint::ZERO);
        node.selected = true;
        let mut model = EditorModel::new(vec![node], vec![], GraphOwnership::Internal).unwrap();
        model.move_selected(WorldPoint::new(3.0, 4.0), None, false);
        assert_eq!(model.nodes[0].position, WorldPoint::new(3.0, 4.0));
        assert!(matches!(
            model.take_changes().0[0],
            NodeChange::Position {
                dragging: Some(false),
                ..
            }
        ));
    }
    #[test]
    fn node_extent_accounts_for_center_origin() {
        let node = Node::new(1u64, WorldPoint::ZERO)
            .with_size(WorldSize::new(10.0, 10.0))
            .with_extent(WorldBounds::new(
                WorldPoint::ZERO,
                WorldSize::new(100.0, 100.0),
            ));
        assert_eq!(
            constrain_node_position(&node, WorldPoint::new(-20.0, 120.0)),
            WorldPoint::new(5.0, 95.0)
        );
    }
    #[test]
    fn controlled_diff_reports_replace_add_and_remove() {
        let old = vec![
            Node::new(1u64, WorldPoint::ZERO),
            Node::new(2u64, WorldPoint::ZERO),
        ];
        let new = vec![
            Node::new(1u64, WorldPoint::new(1.0, 0.0)),
            Node::new(3u64, WorldPoint::ZERO),
        ];
        let changes = diff_node_changes(&old, &new);
        assert!(changes
            .iter()
            .any(|change| matches!(change,NodeChange::Remove{id}if *id==NodeId(2))));
        assert!(changes
            .iter()
            .any(|change| matches!(change,NodeChange::Replace{id,..}if *id==NodeId(1))));
        assert!(changes
            .iter()
            .any(|change| matches!(change,NodeChange::Add{item,..}if item.id==NodeId(3))));
    }
    #[test]
    fn middleware_can_filter_changes() {
        let mut model = EditorModel::new(
            vec![Node::new(1u64, WorldPoint::ZERO)],
            vec![],
            GraphOwnership::Internal,
        )
        .unwrap();
        model.add_node_middleware(|changes| changes.clear());
        model.select_node(NodeId(1), false, false);
        assert!(!model.nodes[0].selected);
    }
    #[test]
    fn controlled_and_internal_ownership_emit_identical_changes() {
        let nodes = vec![Node::new(1u64, WorldPoint::ZERO)];
        let mut internal =
            EditorModel::new(nodes.clone(), vec![], GraphOwnership::Internal).unwrap();
        let mut external = EditorModel::new(nodes, vec![], GraphOwnership::External).unwrap();
        internal.select_node(NodeId(1), false, false);
        external.select_node(NodeId(1), false, false);
        assert_eq!(internal.take_changes(), external.take_changes());
    }
    #[test]
    fn measurement_dirties_only_connected_edges() {
        let nodes = vec![
            Node::new(1u64, WorldPoint::ZERO),
            Node::new(2u64, WorldPoint::ZERO),
            Node::new(3u64, WorldPoint::ZERO),
        ];
        let edges = vec![
            Edge::new(1u64, 2u64).with_id(1u64),
            Edge::new(2u64, 3u64).with_id(2u64),
        ];
        let mut store = EditorStore::default();
        store.rebuild(&nodes, &edges);
        store.take_dirty();
        assert!(store.measure_node(NodeId(1), WorldSize::new(8.0, 8.0), vec![]));
        let dirty = store.take_dirty();
        assert_eq!(dirty.nodes, HashSet::from([NodeId(1)]));
        assert_eq!(dirty.edges, HashSet::from([EdgeId(1)]));
    }

    #[test]
    fn invalid_measurements_leave_runtime_and_dirty_revisions_unchanged() {
        let node = Node::new(1u64, WorldPoint::ZERO).with_size(WorldSize::new(10.0, 20.0));
        let mut store = EditorStore::default();
        store.rebuild(&[node], &[]);
        store.take_dirty();

        let initial_measured = store.runtimes[&NodeId(1)].measured;
        let initial_runtime_revision = store.runtimes[&NodeId(1)].revision;
        let initial_revisions = store.dirty.revisions;

        for size in [
            WorldSize::new(f32::NAN, 20.0),
            WorldSize::new(10.0, f32::INFINITY),
            WorldSize::new(-1.0, 20.0),
            WorldSize::new(10.0, -1.0),
        ] {
            assert!(!store.measure_node(NodeId(1), size, vec![]));
        }

        let runtime = &store.runtimes[&NodeId(1)];
        assert_eq!(runtime.measured, initial_measured);
        assert_eq!(runtime.revision, initial_runtime_revision);
        assert_eq!(store.dirty.revisions, initial_revisions);
        assert_eq!(store.dirty.peek(), &DirtySet::default());
    }
    #[test]
    fn expanding_parent_preserves_child_absolute_position() {
        let parent =
            Node::new(1u64, WorldPoint::new(50.0, 50.0)).with_size(WorldSize::new(100.0, 100.0));
        let mut child = Node::new(2u64, WorldPoint::new(-10.0, 20.0)).with_parent(1u64);
        child.expand_parent = true;
        let nodes = vec![parent, child];
        let mut store = EditorStore::default();
        store.adopt_nodes(&nodes);
        let changes = expand_parent_changes(
            &nodes,
            &store.runtimes,
            NodeId(2),
            WorldBounds::new(WorldPoint::new(-20.0, 10.0), WorldSize::new(10.0, 10.0)),
        );
        assert!(changes
            .iter()
            .any(|change| matches!(change,NodeChange::Dimensions{id,..}if *id==NodeId(1))));
    }

    #[test]
    fn moving_expand_parent_child_emits_parent_changes() {
        let parent =
            Node::new(1u64, WorldPoint::new(50.0, 50.0)).with_size(WorldSize::new(100.0, 100.0));
        let mut child = Node::new(2u64, WorldPoint::new(50.0, 50.0))
            .with_size(WorldSize::new(10.0, 10.0))
            .with_parent(1u64);
        child.expand_parent = true;
        let mut model =
            EditorModel::new(vec![parent, child], vec![], GraphOwnership::Internal).unwrap();
        model.move_nodes(&[(NodeId(2), WorldPoint::new(-20.0, 50.0))], true);
        let (changes, _) = model.take_changes();
        assert!(changes
            .iter()
            .any(|change| matches!(change, NodeChange::Dimensions { id, .. } if *id == NodeId(1))));
    }

    #[test]
    fn reordering_specs_marks_membership_dirty() {
        let nodes = vec![
            Node::new(1u64, WorldPoint::ZERO),
            Node::new(2u64, WorldPoint::ZERO),
        ];
        let mut store = EditorStore::default();
        store.rebuild(&nodes, &[]);
        store.take_dirty();
        store.rebuild(&[nodes[1].clone(), nodes[0].clone()], &[]);
        assert!(store.take_dirty().membership);
    }
}

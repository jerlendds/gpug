//! Incremental invalidation, revision tracking, geometry caching, and culling.
use crate::{Edge, EdgeId, NodeId, NodeRuntime, WorldBounds};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Revisions {
    pub membership: u64,
    /// Advances whenever a node specification changes, independently of
    /// runtime motion and membership.
    pub node_specs: u64,
    /// Advances whenever an edge specification changes, independently of
    /// geometry invalidation and membership.
    pub edge_specs: u64,
    /// Advances whenever any node runtime changes, so consumers can stamp
    /// caches derived from node positions without diffing the dirty set.
    pub nodes: u64,
    /// Advances whenever any edge is invalidated.
    pub edges: u64,
    pub viewport: u64,
    pub selection: u64,
    pub connection: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DirtySet {
    pub nodes: HashSet<NodeId>,
    pub edges: HashSet<EdgeId>,
    pub overlays: bool,
    pub viewport: bool,
    pub membership: bool,
    /// Set when every node and edge moved at once, as during a layout frame.
    /// Per-entity sets stay empty because enumerating them costs more than the
    /// consumer saves; treat `all` as "assume everything changed".
    pub all: bool,
}

#[derive(Default)]
pub struct DirtyTracker {
    pub revisions: Revisions,
    dirty: DirtySet,
    adjacency: HashMap<NodeId, Vec<EdgeId>>,
}

impl DirtyTracker {
    pub fn rebuild(&mut self, edges: &[Edge]) {
        self.adjacency.clear();
        for edge in edges {
            self.adjacency.entry(edge.source).or_default().push(edge.id);
            self.adjacency.entry(edge.target).or_default().push(edge.id);
        }
        self.revisions.membership = self.revisions.membership.wrapping_add(1);
        self.dirty.membership = true;
    }
    pub fn mark_node(&mut self, id: NodeId) {
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.dirty.nodes.insert(id);
        if let Some(edges) = self.adjacency.get(&id) {
            self.revisions.edges = self.revisions.edges.wrapping_add(1);
            self.dirty.edges.extend(edges.iter().copied());
        }
    }
    pub fn mark_node_spec(&mut self) {
        self.revisions.node_specs = self.revisions.node_specs.wrapping_add(1);
    }
    pub fn mark_edge_spec(&mut self) {
        self.revisions.edge_specs = self.revisions.edge_specs.wrapping_add(1);
    }
    pub fn mark_edge(&mut self, id: EdgeId) {
        self.revisions.edges = self.revisions.edges.wrapping_add(1);
        self.dirty.edges.insert(id);
    }
    /// Invalidates every node and edge in one step. A layout frame moves the
    /// whole graph, so per-node adjacency propagation would insert every edge
    /// id into the dirty set each frame while telling consumers nothing they
    /// could not infer from `all`.
    pub fn mark_all(&mut self) {
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.revisions.edges = self.revisions.edges.wrapping_add(1);
        self.dirty.all = true;
    }
    pub fn mark_viewport(&mut self) {
        self.revisions.viewport = self.revisions.viewport.wrapping_add(1);
        self.dirty.viewport = true;
    }
    pub fn mark_selection(&mut self) {
        self.revisions.selection = self.revisions.selection.wrapping_add(1);
        self.dirty.overlays = true;
    }
    pub fn mark_connection(&mut self) {
        self.revisions.connection = self.revisions.connection.wrapping_add(1);
        self.dirty.overlays = true;
    }
    pub fn take(&mut self) -> DirtySet {
        std::mem::take(&mut self.dirty)
    }
    pub fn peek(&self) -> &DirtySet {
        &self.dirty
    }
}

#[derive(Clone, Debug)]
struct CacheEntry<G> {
    edge_revision: u64,
    source_revision: u64,
    target_revision: u64,
    geometry: G,
}
#[derive(Clone, Debug, Default)]
pub struct GeometryCache<G> {
    entries: HashMap<EdgeId, CacheEntry<G>>,
}
impl<G: Clone> GeometryCache<G> {
    pub fn get_or_insert_with(
        &mut self,
        id: EdgeId,
        stamp: (u64, u64, u64),
        compute: impl FnOnce() -> G,
    ) -> G {
        if let Some(entry) = self.entries.get(&id) {
            if (
                entry.edge_revision,
                entry.source_revision,
                entry.target_revision,
            ) == stamp
            {
                return entry.geometry.clone();
            }
        }
        let geometry = compute();
        self.entries.insert(
            id,
            CacheEntry {
                edge_revision: stamp.0,
                source_revision: stamp.1,
                target_revision: stamp.2,
                geometry: geometry.clone(),
            },
        );
        geometry
    }
    pub fn invalidate(&mut self, id: EdgeId) {
        self.entries.remove(&id);
    }
    pub fn retain(&mut self, ids: &HashSet<EdgeId>) {
        self.entries.retain(|id, _| ids.contains(id));
    }
}

/// Allocation-light linear index. Its API permits replacement by an R-tree later.
#[derive(Clone, Debug, Default)]
pub struct VisibilityIndex {
    bounds: HashMap<NodeId, WorldBounds>,
}
impl VisibilityIndex {
    pub fn rebuild(&mut self, runtimes: &HashMap<NodeId, NodeRuntime>) {
        self.bounds.clear();
        self.bounds
            .extend(runtimes.iter().map(|(id, runtime)| (*id, runtime.bounds())));
    }
    pub fn update(&mut self, id: NodeId, bounds: WorldBounds) {
        self.bounds.insert(id, bounds);
    }
    pub fn visible(&self, viewport: WorldBounds) -> impl Iterator<Item = NodeId> + '_ {
        self.bounds
            .iter()
            .filter(move |(_, bounds)| intersects(**bounds, viewport))
            .map(|(id, _)| *id)
    }
}
fn intersects(a: WorldBounds, b: WorldBounds) -> bool {
    a.origin.x + a.size.width >= b.origin.x
        && a.origin.y + a.size.height >= b.origin.y
        && a.origin.x <= b.origin.x + b.size.width
        && a.origin.y <= b.origin.y + b.size.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, NodeId};
    #[test]
    fn moving_one_node_dirties_only_it_and_adjacent_edges() {
        let edges = vec![
            Edge::new(1u64, 2u64).with_id(10u64),
            Edge::new(2u64, 3u64).with_id(11u64),
            Edge::new(4u64, 5u64).with_id(12u64),
        ];
        let mut tracker = DirtyTracker::default();
        tracker.rebuild(&edges);
        tracker.take();
        tracker.mark_node(NodeId(2));
        let dirty = tracker.take();
        assert_eq!(dirty.nodes, HashSet::from([NodeId(2)]));
        assert_eq!(dirty.edges, HashSet::from([EdgeId(10), EdgeId(11)]));
    }
    #[test]
    fn geometry_cache_reuses_matching_revision_stamp() {
        let mut cache = GeometryCache::default();
        let mut computes = 0;
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 2, 3), || {
                computes += 1;
                7
            }),
            7
        );
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 2, 3), || {
                computes += 1;
                8
            }),
            7
        );
        assert_eq!(computes, 1);
        assert_eq!(
            cache.get_or_insert_with(EdgeId(1), (1, 3, 3), || {
                computes += 1;
                9
            }),
            9
        );
        assert_eq!(computes, 2);
    }
}

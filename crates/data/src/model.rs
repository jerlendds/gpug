use crate::column::FrozenColumn;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;
use vortex::array::ArrayRef;

/// Stable caller-assigned identity; never a dense array index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// Stable caller-assigned edge identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeId(pub u64);

impl From<u64> for NodeId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<u64> for EdgeId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Absolute world-space top-left and size. Zero-area rectangles are valid.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WorldRect {
    pub fn is_valid(self) -> bool {
        [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }

    /// Inclusive intersection: touching counts as visible.
    pub fn intersects(self, other: Self) -> bool {
        self.x <= other.x + other.width
            && self.x + self.width >= other.x
            && self.y <= other.y + other.height
            && self.y + self.height >= other.y
    }
}

/// Cold node specification. Renderer keys identify host-provided custom content.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub bounds: WorldRect,
    pub label: String,
    pub renderer: String,
    pub hidden: bool,
}

impl Node {
    pub fn new(id: impl Into<NodeId>, bounds: WorldRect) -> Self {
        Self {
            id: id.into(),
            bounds,
            label: String::new(),
            renderer: "default".into(),
            hidden: false,
        }
    }
}

/// Directed connection. Different handle pairs permit parallel edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub source_handle: Option<String>,
    pub target_handle: Option<String>,
    /// Finite nonnegative analytical weight (zero is allowed).
    pub weight: f64,
}

impl Edge {
    pub fn new(
        id: impl Into<EdgeId>,
        source: impl Into<NodeId>,
        target: impl Into<NodeId>,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            source_handle: None,
            target_handle: None,
            weight: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    DuplicateNode(NodeId),
    DuplicateEdge(EdgeId),
    UnknownNode(NodeId),
    UnknownEdge(EdgeId),
    DuplicateConnection(EdgeId),
    InvalidGeometry(NodeId),
    InvalidWeight(EdgeId),
    TooLarge,
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(f, "duplicate node {}", id.0),
            Self::DuplicateEdge(id) => write!(f, "duplicate edge {}", id.0),
            Self::UnknownNode(id) => write!(f, "unknown node {}", id.0),
            Self::UnknownEdge(id) => write!(f, "unknown edge {}", id.0),
            Self::DuplicateConnection(id) => write!(f, "duplicate connection for edge {}", id.0),
            Self::InvalidGeometry(id) => write!(f, "invalid world bounds for node {}", id.0),
            Self::InvalidWeight(id) => write!(f, "invalid weight for edge {}", id.0),
            Self::TooLarge => write!(f, "graph exceeds dense u32 index capacity"),
        }
    }
}
impl std::error::Error for GraphError {}

/// Ordered batch operations. Removal also removes all incident edges.
#[derive(Clone, Debug)]
pub enum Mutation {
    AddNode(Node),
    ReplaceNode(Node),
    RemoveNode(NodeId),
    AddEdge(Edge),
    ReplaceEdge(Edge),
    RemoveEdge(EdgeId),
}

#[derive(Debug)]
pub(crate) struct Adjacency {
    offsets: FrozenColumn<usize>,
    edges: FrozenColumn<u32>,
}

impl Adjacency {
    fn build(n: usize, endpoints: &[u32]) -> Self {
        let mut offsets = vec![0; n + 1];
        for &node in endpoints {
            offsets[node as usize + 1] += 1;
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut cursors = offsets[..n].to_vec();
        let mut edges = vec![0; endpoints.len()];
        for (edge, &node) in endpoints.iter().enumerate() {
            let cursor = &mut cursors[node as usize];
            edges[*cursor] = edge as u32;
            *cursor += 1;
        }
        Self {
            offsets: offsets.into(),
            edges: edges.into(),
        }
    }

    pub(crate) fn incident(&self, node: usize) -> &[u32] {
        &self.edges[self.offsets[node]..self.offsets[node + 1]]
    }
}

/// Immutable, validated graph. Dense columns and adjacency belong to this snapshot.
///
/// Construction costs O(V + E). Node/edge iteration preserves input order, and
/// adjacency preserves edge order, making traversal deterministic regardless of
/// randomized ID hashing. Hidden nodes remain available to analytical operations.
#[derive(Debug)]
pub struct GraphSnapshot {
    // Vortex tables are canonical. Rows are a lazily materialized compatibility
    // view for existing slice APIs, never a second mutable source of truth.
    tables: OnceLock<(ArrayRef, ArrayRef)>,
    rows: OnceLock<(Vec<Node>, Vec<Edge>)>,
    node_ids: FrozenColumn<u64>,
    edge_ids: FrozenColumn<u64>,
    by_id: HashMap<NodeId, u32>,
    pub(crate) sources: FrozenColumn<u32>,
    pub(crate) targets: FrozenColumn<u32>,
    pub(crate) outgoing: Adjacency,
    pub(crate) incoming: Adjacency,
}

impl GraphSnapshot {
    /// Validate all identities, geometry, endpoints, handles, and weights.
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Result<Self, GraphError> {
        if nodes.len() > u32::MAX as usize || edges.len() > u32::MAX as usize {
            return Err(GraphError::TooLarge);
        }
        let mut by_id = HashMap::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            if !node.bounds.is_valid() {
                return Err(GraphError::InvalidGeometry(node.id));
            }
            if by_id.insert(node.id, index as u32).is_some() {
                return Err(GraphError::DuplicateNode(node.id));
            }
        }
        let mut ids = HashSet::with_capacity(edges.len());
        let mut connections = HashSet::with_capacity(edges.len());
        let mut sources = Vec::with_capacity(edges.len());
        let mut targets = Vec::with_capacity(edges.len());
        for edge in &edges {
            if !ids.insert(edge.id) {
                return Err(GraphError::DuplicateEdge(edge.id));
            }
            if !edge.weight.is_finite() || edge.weight < 0.0 {
                return Err(GraphError::InvalidWeight(edge.id));
            }
            sources.push(
                *by_id
                    .get(&edge.source)
                    .ok_or(GraphError::UnknownNode(edge.source))?,
            );
            targets.push(
                *by_id
                    .get(&edge.target)
                    .ok_or(GraphError::UnknownNode(edge.target))?,
            );
            if !connections.insert((
                edge.source,
                edge.target,
                edge.source_handle.as_deref(),
                edge.target_handle.as_deref(),
            )) {
                return Err(GraphError::DuplicateConnection(edge.id));
            }
        }
        let outgoing = Adjacency::build(nodes.len(), &sources);
        let incoming = Adjacency::build(nodes.len(), &targets);
        let tables = OnceLock::new();
        let rows = OnceLock::new();
        let (node_ids, edge_ids);
        if crate::column::uses_vec_backend() {
            // Explicit benchmark baseline: preserve the former row backing.
            node_ids = nodes
                .iter()
                .map(|node| node.id.0)
                .collect::<Vec<_>>()
                .into();
            edge_ids = edges
                .iter()
                .map(|edge| edge.id.0)
                .collect::<Vec<_>>()
                .into();
            let _ = rows.set((nodes, edges));
        } else {
            let arrays = crate::vortex_io::arrays_from_rows(&nodes, &edges)
                .expect("validated rows produce equal-length fixed-schema Vortex columns");
            let (node_buffer, edge_buffer) =
                crate::vortex_io::identity_buffers(arrays.0.clone(), arrays.1.clone())
                    .expect("internally built Vortex identity columns have u64 buffers");
            node_ids = node_buffer.into();
            edge_ids = edge_buffer.into();
            let _ = tables.set(arrays);
        }
        Ok(Self {
            tables,
            rows,
            node_ids,
            edge_ids,
            by_id,
            sources: sources.into(),
            targets: targets.into(),
            outgoing,
            incoming,
        })
    }

    /// Row compatibility view. First row access materializes both relations in
    /// O(V + E) and retains them; use `node_count`, `endpoint_columns` or
    /// `vortex_tables` when a row representation is unnecessary.
    pub fn nodes(&self) -> &[Node] {
        &self.rows().0
    }
    /// Row compatibility view with the same one-time materialization as `nodes`.
    pub fn edges(&self) -> &[Edge] {
        &self.rows().1
    }

    /// Number of source nodes, without materializing the row compatibility view.
    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }
    /// Number of source edges, without materializing the row compatibility view.
    pub fn edge_count(&self) -> usize {
        self.edge_ids.len()
    }
    pub(crate) fn node_id(&self, index: usize) -> NodeId {
        NodeId(self.node_ids[index])
    }
    pub(crate) fn edge_id(&self, index: usize) -> EdgeId {
        EdgeId(self.edge_ids[index])
    }
    /// Canonical Vortex relations. Cloning an array shares its buffers. This is
    /// O(1) for the production backend; it materializes columns once only for the
    /// explicitly enabled benchmark row backend.
    pub fn vortex_tables(&self) -> &(ArrayRef, ArrayRef) {
        self.tables.get_or_init(|| {
            let (nodes, edges) = self
                .rows
                .get()
                .expect("snapshot has canonical tables or baseline rows");
            crate::vortex_io::arrays_from_rows(nodes, edges)
                .expect("validated snapshot rows have fixed Vortex schemas")
        })
    }

    fn rows(&self) -> &(Vec<Node>, Vec<Edge>) {
        self.rows.get_or_init(|| {
            let (nodes, edges) = self
                .tables
                .get()
                .expect("production snapshot owns Vortex tables");
            crate::vortex_io::VortexGraphIo::rows_from_arrays(nodes.clone(), edges.clone())
                .expect("internally built canonical tables have validated schemas and values")
        })
    }

    pub fn node_index(&self, id: NodeId) -> Option<u32> {
        self.by_id.get(&id).copied()
    }
    /// Endpoint columns suitable for renderer uploads; indices are snapshot-local.
    pub fn endpoint_columns(&self) -> (&[u32], &[u32]) {
        (&self.sources, &self.targets)
    }

    /// Apply an ordered transaction to a new snapshot, leaving this one untouched.
    ///
    /// Validation uses the final projected graph, allowing edges to be added before
    /// their endpoints within a batch. Unknown mutation targets fail the batch.
    /// Copy/rebuild costs O(V + E), plus linear target lookup per operation.
    /// Intended for workers, never frame construction or high-frequency dragging.
    pub fn transact(
        &self,
        changes: impl IntoIterator<Item = Mutation>,
    ) -> Result<Self, GraphError> {
        let mut nodes = self.nodes().to_vec();
        let mut edges = self.edges().to_vec();
        for change in changes {
            match change {
                Mutation::AddNode(node) => {
                    if nodes.iter().any(|n| n.id == node.id) {
                        return Err(GraphError::DuplicateNode(node.id));
                    }
                    nodes.push(node);
                }
                Mutation::ReplaceNode(node) => {
                    let old = nodes
                        .iter_mut()
                        .find(|n| n.id == node.id)
                        .ok_or(GraphError::UnknownNode(node.id))?;
                    *old = node;
                }
                Mutation::RemoveNode(id) => {
                    let index = nodes
                        .iter()
                        .position(|n| n.id == id)
                        .ok_or(GraphError::UnknownNode(id))?;
                    nodes.remove(index);
                    edges.retain(|e| e.source != id && e.target != id);
                }
                Mutation::AddEdge(edge) => {
                    if edges.iter().any(|e| e.id == edge.id) {
                        return Err(GraphError::DuplicateEdge(edge.id));
                    }
                    edges.push(edge);
                }
                Mutation::ReplaceEdge(edge) => {
                    let old = edges
                        .iter_mut()
                        .find(|e| e.id == edge.id)
                        .ok_or(GraphError::UnknownEdge(edge.id))?;
                    *old = edge;
                }
                Mutation::RemoveEdge(id) => {
                    let index = edges
                        .iter()
                        .position(|e| e.id == id)
                        .ok_or(GraphError::UnknownEdge(id))?;
                    edges.remove(index);
                }
            }
        }
        Self::new(nodes, edges)
    }

    /// Exact induced subgraph in original input order. Unknown IDs are errors.
    pub fn induced_subgraph(&self, ids: &[NodeId]) -> Result<Self, GraphError> {
        let mut selected = vec![false; self.nodes().len()];
        for &id in ids {
            selected[self.node_index(id).ok_or(GraphError::UnknownNode(id))? as usize] = true;
        }
        let nodes = self
            .nodes()
            .iter()
            .enumerate()
            .filter(|(i, _)| selected[*i])
            .map(|(_, n)| n.clone())
            .collect();
        let edges = self
            .edges()
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                selected[self.sources[*i] as usize] && selected[self.targets[*i] as usize]
            })
            .map(|(_, e)| e.clone())
            .collect();
        Self::new(nodes, edges)
    }
}

#[cfg(test)]
mod backing_tests {
    use super::*;
    use crate::{Direction, SearchBudget, SearchWorkspace};

    #[test]
    fn canonical_export_shares_arrays_and_traversal_does_not_materialize_rows() {
        let graph = GraphSnapshot::new(
            vec![
                Node::new(1, WorldRect::default()),
                Node::new(2, WorldRect::default()),
            ],
            vec![Edge::new(10, 1, 2)],
        )
        .unwrap();
        let mut search = SearchWorkspace::new();
        let found = search
            .neighborhood(
                &graph,
                NodeId(1),
                Direction::Outgoing,
                SearchBudget::default(),
            )
            .unwrap();
        assert_eq!(found.nodes, &[NodeId(1), NodeId(2)]);
        if !crate::column::uses_vec_backend() {
            assert!(graph.rows.get().is_none());
            assert!(graph.tables.get().is_some());
        }
        let exported = crate::vortex_io::graph_arrays(&graph).unwrap();
        assert!(ArrayRef::ptr_eq(&exported.0, &graph.vortex_tables().0));
        assert!(ArrayRef::ptr_eq(&exported.1, &graph.vortex_tables().1));
        assert_eq!(graph.nodes()[1].id, NodeId(2));
        assert_eq!(graph.edges()[0].id, EdgeId(10));
    }
}

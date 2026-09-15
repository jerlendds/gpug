//! Immutable node-bounds indexing. Build on a worker and reuse query scratch space.
//! This index does not cull edges: an edge can cross a viewport with both endpoints
//! outside it, and renderer paths can extend beyond their endpoint rectangles.
use crate::{GraphSnapshot, NodeId, WorldRect};
use std::fmt;
use std::sync::Arc;

mod connectivity;
mod lod;
pub use connectivity::{
    ConnectionBudget, ConnectionCache, ConnectionCacheStats, ConnectionGroup, ConnectionPlanError,
    PlanConnections, PlanEndpoint,
};
pub use lod::{
    InvalidLodOptions, LodItem, LodKind, LodOptions, LodPlan, LodWorkspace, OwnedLodPlan,
};

const LEAF_SIZE: usize = 16;

#[cfg(all(test, feature = "gpug"))]
mod aggregate_identity_tests {
    use super::*;
    use crate::Node;
    use std::collections::HashSet;

    #[test]
    fn reserved_block_excludes_all_source_ids_including_hidden_extremes() {
        for count in [1u64, 2, 17, 257] {
            for pattern in 0..4 {
                let ids = (0..count)
                    .map(|i| match pattern {
                        0 => i,
                        1 => i * count, // occupies every explicitly tracked block
                        2 => u64::MAX - i,
                        _ => {
                            if i % 2 == 0 {
                                i * count
                            } else {
                                u64::MAX - i
                            }
                        }
                    })
                    .collect::<HashSet<_>>();
                let nodes = ids
                    .iter()
                    .map(|&id| {
                        let mut node = Node::new(id, WorldRect::default());
                        node.hidden = id % 2 == 0;
                        node
                    })
                    .collect();
                let index = SpatialIndex::new(Arc::new(GraphSnapshot::new(nodes, vec![]).unwrap()));
                let mut reserved = HashSet::new();
                for ordinal in 0..count as usize {
                    let id = index.aggregate_id(ordinal).0;
                    assert!(!ids.contains(&id));
                    assert!(reserved.insert(id));
                }
            }
        }
    }
}

// Endpoints avoid overflow when the union spans almost the entire f64 range.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}
impl From<WorldRect> for Bounds {
    fn from(rect: WorldRect) -> Self {
        Self {
            min_x: rect.x,
            min_y: rect.y,
            max_x: rect.x + rect.width,
            max_y: rect.y + rect.height,
        }
    }
}
impl Bounds {
    fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }
    fn intersects(self, other: Self) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }
}
#[derive(Debug)]
enum Children {
    Leaf { start: usize, end: usize },
    Split { left: usize, right: usize },
}
#[derive(Debug)]
struct Branch {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Children,
}

/// Balanced bounding-volume hierarchy over non-hidden node rectangles. The owned
/// Arc binds every branch and dense index to one immutable source snapshot.
/// Rebuild after geometry, membership or hidden-state changes; do not use this
/// index to cull a newer/moving GPUG editor state. Construction is O(V log V).
#[derive(Debug)]
pub struct SpatialIndex {
    graph: Arc<GraphSnapshot>,
    indices: Vec<u32>,
    // Dense source index -> membership order, enabling bounded focus extraction.
    member_positions: Vec<u32>,
    #[cfg(feature = "gpug")]
    aggregate_id_base: u64,
    branches: Vec<Branch>,
}
impl SpatialIndex {
    pub fn new(graph: Arc<GraphSnapshot>) -> Self {
        // N source IDs occupy at most N disjoint blocks of N identities. One of
        // the first N+1 blocks must be empty. N <= u32::MAX, so every candidate
        // block fits u64. Reserve N IDs once, rather than searching on each view.
        #[cfg(feature = "gpug")]
        let count = graph.nodes().len();
        #[cfg(feature = "gpug")]
        let aggregate_id_base = if count == 0 {
            0
        } else {
            let mut occupied = vec![false; count];
            for node in graph.nodes() {
                let block = node.id.0 / count as u64;
                if block < count as u64 {
                    occupied[block as usize] = true;
                }
            }
            occupied
                .iter()
                .position(|occupied| !occupied)
                .unwrap_or(count) as u64
                * count as u64
        };
        let mut indices: Vec<u32> = graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(_, node)| !node.hidden)
            .map(|(index, _)| {
                u32::try_from(index).expect("snapshot validates dense index capacity")
            })
            .collect();
        let mut branches = Vec::with_capacity(2 * indices.len().div_ceil(LEAF_SIZE));
        if !indices.is_empty() {
            build(&graph, &mut indices, 0, &mut branches);
        }
        let mut member_positions = vec![u32::MAX; graph.nodes().len()];
        for (position, &dense) in indices.iter().enumerate() {
            member_positions[dense as usize] = position as u32;
        }
        Self {
            graph,
            indices,
            member_positions,
            #[cfg(feature = "gpug")]
            aggregate_id_base,
            branches,
        }
    }
    pub fn snapshot(&self) -> &Arc<GraphSnapshot> {
        &self.graph
    }
    #[cfg(feature = "gpug")]
    pub(crate) fn aggregate_id(&self, ordinal: usize) -> NodeId {
        // Disjoint nonempty plan membership permits at most N aggregates.
        assert!(ordinal < self.graph.nodes().len());
        NodeId(self.aggregate_id_base + ordinal as u64)
    }
    pub fn indexed_nodes(&self) -> usize {
        self.indices.len()
    }
    /// Allocated index buffers, excluding the shared snapshot and allocator overhead.
    pub fn allocated_bytes(&self) -> usize {
        self.indices.capacity() * std::mem::size_of::<u32>()
            + self.member_positions.capacity() * std::mem::size_of::<u32>()
            + self.branches.capacity() * std::mem::size_of::<Branch>()
    }
}

fn build(
    graph: &GraphSnapshot,
    indices: &mut [u32],
    offset: usize,
    branches: &mut Vec<Branch>,
) -> usize {
    let mut bounds = Bounds::from(graph.nodes()[indices[0] as usize].bounds);
    for &index in &indices[1..] {
        bounds = bounds.union(graph.nodes()[index as usize].bounds.into());
    }
    let slot = branches.len();
    branches.push(Branch {
        bounds,
        start: offset,
        end: offset + indices.len(),
        children: Children::Leaf {
            start: offset,
            end: offset + indices.len(),
        },
    });
    if indices.len() > LEAF_SIZE {
        // An infinite span is only a split-axis heuristic, never stored geometry.
        let x_axis = bounds.max_x - bounds.min_x >= bounds.max_y - bounds.min_y;
        let middle = indices.len() / 2;
        indices.select_nth_unstable_by(middle, |a, b| {
            let a_node = &graph.nodes()[*a as usize];
            let b_node = &graph.nodes()[*b as usize];
            let center = |rect: WorldRect| {
                if x_axis {
                    rect.x + rect.width * 0.5
                } else {
                    rect.y + rect.height * 0.5
                }
            };
            center(a_node.bounds)
                .total_cmp(&center(b_node.bounds))
                .then_with(|| a_node.id.cmp(&b_node.id))
        });
        let (left_indices, right_indices) = indices.split_at_mut(middle);
        let left = build(graph, left_indices, offset, branches);
        let right = build(graph, right_indices, offset + middle, branches);
        branches[slot].children = Children::Split { left, right };
    }
    slot
}

#[derive(Clone, Copy, Debug)]
pub struct SpatialBudget {
    pub max_branches: usize,
    pub max_nodes: usize,
    pub max_results: usize,
}
impl Default for SpatialBudget {
    fn default() -> Self {
        Self {
            max_branches: 10_000,
            max_nodes: 100_000,
            max_results: 10_000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidViewport;
impl fmt::Display for InvalidViewport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("viewport bounds must be finite and nonnegative in size")
    }
}
impl std::error::Error for InvalidViewport {}

/// Results are in hierarchy traversal order, not source order. Inclusive touching
/// counts as intersection. Truncation means this is NOT a complete visibility set:
/// retain a conservative fallback/aggregate representation for unresolved regions.
#[derive(Debug)]
pub struct SpatialResult<'a> {
    pub nodes: &'a [NodeId],
    pub visited_branches: usize,
    pub examined_nodes: usize,
    pub truncated: bool,
}
#[derive(Debug, Default)]
pub struct SpatialWorkspace {
    stack: Vec<usize>,
    nodes: Vec<NodeId>,
}
impl SpatialWorkspace {
    pub fn new() -> Self {
        Self::default()
    }
    /// Query a snapshot's node rectangles, excluding hidden nodes. No padding is
    /// implicit: expand the viewport to account for strokes/labels when needed.
    /// Empty and zero-area viewports are valid; invalid bounds fail before reuse.
    pub fn query<'a>(
        &'a mut self,
        index: &SpatialIndex,
        viewport: WorldRect,
        budget: SpatialBudget,
    ) -> Result<SpatialResult<'a>, InvalidViewport> {
        if !viewport.is_valid() {
            return Err(InvalidViewport);
        }
        self.stack.clear();
        self.nodes.clear();
        if !index.branches.is_empty() {
            self.stack.push(0);
        }
        let viewport = Bounds::from(viewport);
        let mut visited_branches = 0;
        let mut examined_nodes = 0;
        let mut truncated = false;
        'query: while let Some(branch) = self.stack.pop() {
            if visited_branches == budget.max_branches {
                truncated = true;
                break;
            }
            visited_branches += 1;
            let branch = &index.branches[branch];
            if !branch.bounds.intersects(viewport) {
                continue;
            }
            match branch.children {
                Children::Split { left, right } => {
                    self.stack.push(right);
                    self.stack.push(left);
                }
                Children::Leaf { start, end } => {
                    for &dense in &index.indices[start..end] {
                        if examined_nodes == budget.max_nodes {
                            truncated = true;
                            break 'query;
                        }
                        examined_nodes += 1;
                        let node = &index.graph.nodes()[dense as usize];
                        if Bounds::from(node.bounds).intersects(viewport) {
                            if self.nodes.len() == budget.max_results {
                                truncated = true;
                                break 'query;
                            }
                            self.nodes.push(node.id);
                        }
                    }
                }
            }
        }
        Ok(SpatialResult {
            nodes: &self.nodes,
            visited_branches,
            examined_nodes,
            truncated,
        })
    }
}

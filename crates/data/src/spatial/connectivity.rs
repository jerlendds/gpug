use super::OwnedLodPlan;
use crate::EdgeId;
use std::{collections::HashMap, fmt, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanEndpoint {
    /// Index into this connection plan's node-plan items, never a source NodeId.
    Item(usize),
    /// Non-hidden endpoint not represented by the node plan. This does NOT mean
    /// its edge is invisible: both endpoints may lie outside a crossed viewport.
    Outside,
}
#[derive(Clone, Copy, Debug)]
pub struct ConnectionBudget {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_groups: usize,
}
impl Default for ConnectionBudget {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_edges: 10_000_000,
            max_groups: 10_000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionPlanError {
    NodeLimit,
    EdgeLimit,
    GroupLimit,
    WeightOverflow,
    Cancelled,
}
impl fmt::Display for ConnectionPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NodeLimit => "connection planning node limit exceeded",
            Self::EdgeLimit => "connection planning edge limit exceeded",
            Self::GroupLimit => "connection planning group limit exceeded",
            Self::WeightOverflow => "aggregate edge weight overflow",
            Self::Cancelled => "connection planning cancelled",
        })
    }
}
impl std::error::Error for ConnectionPlanError {}

#[derive(Debug)]
pub struct ConnectionGroup {
    pub source: PlanEndpoint,
    pub target: PlanEndpoint,
    /// Includes self-loops and handle-distinct parallel connections.
    pub edge_count: usize,
    /// Sum in source order; normal f64 rounding applies. Overflow is an error.
    pub weight_sum: f64,
}

/// Complete directed source connectivity grouped by node-plan endpoint owners.
/// Edges incident to hidden nodes are counted separately. No geometric edge
/// culling is inferred. Outside/Outside groups must remain explicit in a view;
/// they include potentially visible crossing edges. A renderer needs a separate
/// conservative path query to draw their actual geometry.
#[derive(Clone, Debug)]
pub struct PlanConnections {
    plan: Arc<OwnedLodPlan>,
    data: Arc<ConnectionData>,
    pub hidden_edges: usize,
}
#[derive(Debug)]
struct ConnectionData {
    owners: Vec<usize>,
    groups: Vec<ConnectionGroup>,
}
impl PlanConnections {
    /// Resolve a stable source node to its current plan item in constant expected
    /// time. Unknown, hidden and unrepresented nodes return None. Useful for
    /// retaining source selection across changes of LOD representation.
    pub fn node_item(&self, id: crate::NodeId) -> Option<usize> {
        let dense = self.plan.index.snapshot().node_index(id)? as usize;
        let owner = self.data.owners[dense];
        (owner != usize::MAX).then_some(owner)
    }
    pub fn plan(&self) -> &Arc<OwnedLodPlan> {
        &self.plan
    }
    pub fn groups(&self) -> &[ConnectionGroup] {
        &self.data.groups
    }
    /// Lazily scan original edges to resolve one group's exact IDs. This is O(E),
    /// intended for worker drill-down, not per-frame painting or pointer handling.
    pub fn members(&self, group: usize) -> Option<impl Iterator<Item = EdgeId> + '_> {
        let group = self.data.groups.get(group)?;
        let graph = self.plan.index.snapshot();
        let (sources, targets) = graph.endpoint_columns();
        Some(
            graph
                .edges()
                .iter()
                .enumerate()
                .filter_map(move |(index, edge)| {
                    let source = sources[index] as usize;
                    let target = targets[index] as usize;
                    if graph.nodes()[source].hidden || graph.nodes()[target].hidden {
                        return None;
                    }
                    (endpoint(self.data.owners[source]) == group.source
                        && endpoint(self.data.owners[target]) == group.target)
                        .then_some(edge.id)
                }),
        )
    }
}
fn endpoint(owner: usize) -> PlanEndpoint {
    if owner == usize::MAX {
        PlanEndpoint::Outside
    } else {
        PlanEndpoint::Item(owner)
    }
}

// LOD usually has only a few hundred owners. A bounded dense pair table avoids
// hashing the same endpoint pairs millions of times, preserving first-seen group
// order and source-order floating point addition. Large/sparse domains fall back
// to a hash map rather than allocating quadratic unbounded storage.
enum GroupLookup {
    Dense { width: usize, groups: Vec<usize> },
    Sparse(HashMap<(usize, usize), usize>),
}
impl GroupLookup {
    fn new(items: usize, edges: usize) -> Self {
        let slots = items
            .checked_add(1)
            .and_then(|width| width.checked_mul(width));
        match slots {
            Some(slots) if slots <= 1_048_576 && slots <= edges.saturating_mul(4) => Self::Dense {
                width: items + 1,
                groups: vec![usize::MAX; slots],
            },
            _ => Self::Sparse(HashMap::new()),
        }
    }

    fn slot(&mut self, source: usize, target: usize, outside: usize) -> &mut usize {
        let source = if source == usize::MAX {
            outside
        } else {
            source
        };
        let target = if target == usize::MAX {
            outside
        } else {
            target
        };
        match self {
            Self::Dense { width, groups } => &mut groups[source * *width + target],
            Self::Sparse(groups) => groups.entry((source, target)).or_insert(usize::MAX),
        }
    }
}

impl OwnedLodPlan {
    /// Build an exact summary on a worker in O(V + E), with an O(V) ownership
    /// table and at most max_groups descriptors. Small owner domains use a dense
    /// pair lookup capped at 1,048,576 usize entries (8 MiB on 64-bit targets);
    /// large/sparse domains use a hash map. Whole
    /// node/edge limits are checked before allocation; group-limit/overflow/cancel
    /// failures publish no partial summary. Use ConnectionCache for partition reuse.
    /// Cancellation is checked before setup and every 256 assigned nodes/edges;
    /// initial ownership/pair-table allocations are indivisible.
    pub fn connections(
        self: &Arc<Self>,
        budget: ConnectionBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PlanConnections, ConnectionPlanError> {
        let graph = self.index.snapshot();
        if graph.nodes().len() > budget.max_nodes {
            return Err(ConnectionPlanError::NodeLimit);
        }
        if graph.edges().len() > budget.max_edges {
            return Err(ConnectionPlanError::EdgeLimit);
        }
        if cancelled() {
            return Err(ConnectionPlanError::Cancelled);
        }
        let mut owners = vec![usize::MAX; graph.nodes().len()];
        let mut assigned = 0usize;
        for (owner, item) in self.items.iter().enumerate() {
            for &dense in &self.index.indices[item.members.clone()] {
                if assigned.is_multiple_of(256) && cancelled() {
                    return Err(ConnectionPlanError::Cancelled);
                }
                owners[dense as usize] = owner;
                assigned += 1;
            }
        }
        let mut lookup = GroupLookup::new(self.items.len(), graph.edges().len());
        let mut groups: Vec<ConnectionGroup> = Vec::new();
        let mut hidden_edges = 0;
        let (sources, targets) = graph.endpoint_columns();
        for (index, edge) in graph.edges().iter().enumerate() {
            if index.is_multiple_of(256) && cancelled() {
                return Err(ConnectionPlanError::Cancelled);
            }
            let source = sources[index] as usize;
            let target = targets[index] as usize;
            if graph.nodes()[source].hidden || graph.nodes()[target].hidden {
                hidden_edges += 1;
                continue;
            }
            let slot = lookup.slot(owners[source], owners[target], self.items.len());
            let group = match *slot {
                usize::MAX => {
                    if groups.len() == budget.max_groups {
                        return Err(ConnectionPlanError::GroupLimit);
                    }
                    let group = groups.len();
                    groups.push(ConnectionGroup {
                        source: endpoint(owners[source]),
                        target: endpoint(owners[target]),
                        edge_count: 0,
                        weight_sum: 0.,
                    });
                    *slot = group;
                    group
                }
                group => group,
            };
            let aggregate = &mut groups[group];
            let weight = aggregate.weight_sum + edge.weight;
            if !weight.is_finite() {
                return Err(ConnectionPlanError::WeightOverflow);
            }
            aggregate.weight_sum = weight;
            aggregate.edge_count += 1;
        }
        Ok(PlanConnections {
            plan: Arc::clone(self),
            data: Arc::new(ConnectionData { owners, groups }),
            hidden_edges,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectionCacheStats {
    pub hits: u64,
    pub builds: u64,
}

/// One-entry worker-owned cache. Reuses exact source connectivity when the same
/// immutable index and ordered membership ranges recur. View bounds and Node vs
/// Aggregate presentation may change without invalidating connectivity. A hit
/// shares the O(V) owner table and group buffers, binding them to the new plan.
/// Failed/cancelled requests retain the prior successful entry. Consumers holding
/// older results retain their buffers independently of this cache's one-entry cap.
#[derive(Debug, Default)]
pub struct ConnectionCache {
    latest: Option<PlanConnections>,
    stats: ConnectionCacheStats,
}
impl ConnectionCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stats(&self) -> ConnectionCacheStats {
        self.stats
    }
    /// Drop the retained entry; previously published results remain valid.
    pub fn clear(&mut self) {
        self.latest = None;
    }
    /// O(items) on a hit, O(V+E) on a miss. All requested source/group limits still
    /// apply on hits. Cancellation is checked before work and during comparisons.
    /// No hashes, source revisions supplied by the caller, or approximate bounds
    /// comparisons are used as proof of cache identity.
    pub fn prepare(
        &mut self,
        plan: Arc<OwnedLodPlan>,
        budget: ConnectionBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PlanConnections, ConnectionPlanError> {
        let graph = plan.index.snapshot();
        if graph.nodes().len() > budget.max_nodes {
            return Err(ConnectionPlanError::NodeLimit);
        }
        if graph.edges().len() > budget.max_edges {
            return Err(ConnectionPlanError::EdgeLimit);
        }
        if cancelled() {
            return Err(ConnectionPlanError::Cancelled);
        }
        if let Some(previous) = &self.latest {
            let mut same = Arc::ptr_eq(&previous.plan.index, &plan.index)
                && previous.plan.items.len() == plan.items.len();
            if same {
                for (index, (a, b)) in previous.plan.items.iter().zip(&plan.items).enumerate() {
                    if index.is_multiple_of(256) && cancelled() {
                        return Err(ConnectionPlanError::Cancelled);
                    }
                    if a.members != b.members {
                        same = false;
                        break;
                    }
                }
            }
            if same {
                if previous.groups().len() > budget.max_groups {
                    return Err(ConnectionPlanError::GroupLimit);
                }
                let result = PlanConnections {
                    plan,
                    data: Arc::clone(&previous.data),
                    hidden_edges: previous.hidden_edges,
                };
                self.latest = Some(result.clone());
                self.stats.hits = self.stats.hits.saturating_add(1);
                return Ok(result);
            }
        }
        let result = plan.connections(budget, cancelled)?;
        self.latest = Some(result.clone());
        self.stats.builds = self.stats.builds.saturating_add(1);
        Ok(result)
    }
}

use crate::{GraphError, GraphSnapshot, NodeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

/// Deterministic work limits, including the seed in `max_nodes`.
/// `max_edges` counts adjacency entries examined (a self-loop may count twice
/// in `Both` mode). A hop boundary is part of the query, not truncation.
#[derive(Clone, Copy, Debug)]
pub struct SearchBudget {
    pub max_hops: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
}

impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            max_hops: 1,
            max_nodes: 10_000,
            max_edges: 100_000,
        }
    }
}

/// Borrowed BFS result, valid until the workspace is reused.
#[derive(Debug)]
pub struct Neighborhood<'a> {
    pub nodes: &'a [NodeId],
    /// Dense node indices aligned with `nodes`; valid only for the queried snapshot.
    pub indices: &'a [u32],
    pub depths: &'a [usize],
    pub examined_edges: usize,
    /// Work was stopped by a node/edge budget or cancellation; never treat this as exhaustive.
    pub truncated: bool,
}

/// Retains visited marks and queue capacity across interactive neighborhood queries.
/// After initial sizing, a small query does not clear all V visited entries.
#[derive(Debug, Default)]
pub struct SearchWorkspace {
    marks: Vec<u64>,
    epoch: u64,
    queue: Vec<u32>,
    ids: Vec<NodeId>,
    depths: Vec<usize>,
}

impl SearchWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Search exact directed hop distances until the explicit budget is exhausted.
    /// Hidden nodes participate because visibility does not change graph topology.
    /// Unknown seeds fail before altering the workspace.
    pub fn neighborhood<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        seed: NodeId,
        direction: Direction,
        budget: SearchBudget,
    ) -> Result<Neighborhood<'a>, GraphError> {
        self.neighborhood_cancellable(graph, seed, direction, budget, || false)
    }

    /// Cooperative variant; checks cancellation before setup and every 256
    /// adjacency entries. A cancelled result is marked truncated. The callback
    /// should be cheap/nonblocking; workspace growth is not interruptible.
    pub fn neighborhood_cancellable<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        seed: NodeId,
        direction: Direction,
        budget: SearchBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Neighborhood<'a>, GraphError> {
        let start = graph
            .node_index(seed)
            .ok_or(GraphError::UnknownNode(seed))?;
        if cancelled() {
            self.queue.clear();
            self.ids.clear();
            self.depths.clear();
            return Ok(Neighborhood {
                nodes: &self.ids,
                indices: &self.queue,
                depths: &self.depths,
                examined_edges: 0,
                truncated: true,
            });
        }
        self.marks.resize(graph.nodes().len(), 0);
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.epoch = 1;
        }
        self.queue.clear();
        self.ids.clear();
        self.depths.clear();
        let mut examined_edges = 0;
        let mut truncated = budget.max_nodes == 0;
        if !truncated {
            self.queue.push(start);
            self.ids.push(seed);
            self.depths.push(0);
            self.marks[start as usize] = self.epoch;
        }
        let mut head = 0;
        'search: while head < self.queue.len() {
            let node = self.queue[head] as usize;
            let depth = self.depths[head];
            head += 1;
            if depth == budget.max_hops {
                continue;
            }
            let outgoing = if direction != Direction::Incoming {
                graph.outgoing.incident(node)
            } else {
                &[]
            };
            let incoming = if direction != Direction::Outgoing {
                graph.incoming.incident(node)
            } else {
                &[]
            };
            for (entries, endpoints) in [(outgoing, &graph.targets), (incoming, &graph.sources)] {
                for &edge in entries {
                    if examined_edges % 256 == 0 && cancelled() {
                        truncated = true;
                        break 'search;
                    }
                    if examined_edges == budget.max_edges {
                        truncated = true;
                        break 'search;
                    }
                    examined_edges += 1;
                    let next = endpoints[edge as usize];
                    if self.marks[next as usize] == self.epoch {
                        continue;
                    }
                    if self.queue.len() == budget.max_nodes {
                        truncated = true;
                        break 'search;
                    }
                    self.marks[next as usize] = self.epoch;
                    self.queue.push(next);
                    self.ids.push(graph.nodes()[next as usize].id);
                    self.depths.push(depth + 1);
                }
            }
        }
        Ok(Neighborhood {
            nodes: &self.ids,
            indices: &self.queue,
            depths: &self.depths,
            examined_edges,
            truncated,
        })
    }
}

impl GraphSnapshot {
    /// Returns (in-degree, out-degree), counting handle-distinct parallel edges.
    /// Self-loops contribute one to each degree.
    pub fn degree(&self, node: NodeId) -> Result<(usize, usize), GraphError> {
        let index = self.node_index(node).ok_or(GraphError::UnknownNode(node))? as usize;
        Ok((
            self.incoming.incident(index).len(),
            self.outgoing.incident(index).len(),
        ))
    }

    /// Weakly connected component labels in dense node order, O(V + E).
    /// Components are numbered by first node encountered in input order.
    /// This whole-graph analytical operation belongs on a worker thread.
    pub fn weak_components(&self) -> Vec<u32> {
        let mut labels = vec![u32::MAX; self.nodes().len()];
        let mut queue = Vec::new();
        let mut component = 0;
        for seed in 0..labels.len() {
            if labels[seed] != u32::MAX {
                continue;
            }
            labels[seed] = component;
            queue.clear();
            queue.push(seed);
            let mut head = 0;
            while head < queue.len() {
                let node = queue[head];
                head += 1;
                for (entries, endpoints) in [
                    (self.outgoing.incident(node), &self.targets),
                    (self.incoming.incident(node), &self.sources),
                ] {
                    for &edge in entries {
                        let next = endpoints[edge as usize] as usize;
                        if labels[next] == u32::MAX {
                            labels[next] = component;
                            queue.push(next);
                        }
                    }
                }
            }
            component += 1;
        }
        labels
    }
}

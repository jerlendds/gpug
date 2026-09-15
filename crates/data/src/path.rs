use crate::{Direction, EdgeId, GraphError, GraphSnapshot, NodeId, SearchBudget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStop {
    NodeBudget,
    EdgeBudget,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatus {
    Found,
    /// No path exists within the requested hop limit, not necessarily globally.
    NotFoundWithinHops,
    /// No conclusion about reachability may be drawn.
    Stopped(PathStop),
}

/// A minimum-hop path over source topology. Weights do not affect this search.
/// Empty buffers accompany every status except Found. Edges align with successive
/// node pairs; in Incoming/Both mode an edge may be traversed against its direction.
#[derive(Debug)]
pub struct PathResult<'a> {
    pub status: PathStatus,
    pub nodes: &'a [NodeId],
    pub edges: &'a [EdgeId],
    pub visited_nodes: usize,
    pub examined_edges: usize,
}

/// Reuses epoch marks and predecessor arrays. Initial sizing is O(V), warm searches
/// touch only the explored working set. Intended for a data worker.
#[derive(Debug, Default)]
pub struct PathWorkspace {
    marks: Vec<u64>,
    epoch: u64,
    parents: Vec<u32>,
    parent_edges: Vec<u32>,
    queue: Vec<(u32, usize)>,
    nodes: Vec<NodeId>,
    edges: Vec<EdgeId>,
}

impl PathWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a minimum-hop path, breaking ties by snapshot adjacency order
    /// (outgoing before incoming in Both mode). Hidden nodes participate.
    /// Both endpoints are validated before modifying the workspace. The seed
    /// counts toward max_nodes; max_edges counts examined adjacency entries.
    pub fn shortest_path<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        source: NodeId,
        target: NodeId,
        direction: Direction,
        budget: SearchBudget,
    ) -> Result<PathResult<'a>, GraphError> {
        self.shortest_path_cancellable(graph, source, target, direction, budget, || false)
    }

    /// Checks cancellation before setup and every 256 adjacency entries or path
    /// reconstruction steps. Array growth is indivisible. Cancellation never
    /// returns a partial path; the buffers are empty and status is Stopped.
    pub fn shortest_path_cancellable<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        source: NodeId,
        target: NodeId,
        direction: Direction,
        budget: SearchBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PathResult<'a>, GraphError> {
        let start = graph
            .node_index(source)
            .ok_or(GraphError::UnknownNode(source))?;
        let goal = graph
            .node_index(target)
            .ok_or(GraphError::UnknownNode(target))?;
        self.queue.clear();
        self.nodes.clear();
        self.edges.clear();
        if cancelled() {
            return Ok(self.result(PathStatus::Stopped(PathStop::Cancelled), 0));
        }
        if budget.max_nodes == 0 {
            return Ok(self.result(PathStatus::Stopped(PathStop::NodeBudget), 0));
        }
        self.marks.resize(graph.node_count(), 0);
        self.parents.resize(graph.node_count(), 0);
        self.parent_edges.resize(graph.node_count(), 0);
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.epoch = 1;
        }
        self.marks[start as usize] = self.epoch;
        self.queue.push((start, 0));
        let mut examined = 0;
        let mut found = start == goal;
        let mut status = PathStatus::NotFoundWithinHops;
        let mut head = 0;
        'search: while !found && head < self.queue.len() {
            let (node, depth) = self.queue[head];
            head += 1;
            if depth == budget.max_hops {
                continue;
            }
            let outgoing = if direction == Direction::Incoming {
                &[]
            } else {
                graph.outgoing.incident(node as usize)
            };
            let incoming = if direction == Direction::Outgoing {
                &[]
            } else {
                graph.incoming.incident(node as usize)
            };
            for (entries, endpoints) in [(outgoing, &graph.targets), (incoming, &graph.sources)] {
                for &edge in entries {
                    if examined % 256 == 0 && cancelled() {
                        status = PathStatus::Stopped(PathStop::Cancelled);
                        break 'search;
                    }
                    if examined == budget.max_edges {
                        status = PathStatus::Stopped(PathStop::EdgeBudget);
                        break 'search;
                    }
                    examined += 1;
                    let next = endpoints[edge as usize];
                    if self.marks[next as usize] == self.epoch {
                        continue;
                    }
                    if self.queue.len() == budget.max_nodes {
                        status = PathStatus::Stopped(PathStop::NodeBudget);
                        break 'search;
                    }
                    self.marks[next as usize] = self.epoch;
                    self.parents[next as usize] = node;
                    self.parent_edges[next as usize] = edge;
                    self.queue.push((next, depth + 1));
                    if next == goal {
                        found = true;
                        break 'search;
                    }
                }
            }
        }
        if found {
            let mut node = goal;
            loop {
                if self.nodes.len().is_multiple_of(256) && cancelled() {
                    self.nodes.clear();
                    self.edges.clear();
                    return Ok(self.result(PathStatus::Stopped(PathStop::Cancelled), examined));
                }
                self.nodes.push(graph.node_id(node as usize));
                if node == start {
                    break;
                }
                self.edges
                    .push(graph.edge_id(self.parent_edges[node as usize] as usize));
                node = self.parents[node as usize];
            }
            self.nodes.reverse();
            self.edges.reverse();
            status = PathStatus::Found;
        }
        Ok(self.result(status, examined))
    }

    fn result(&self, status: PathStatus, examined_edges: usize) -> PathResult<'_> {
        PathResult {
            status,
            nodes: &self.nodes,
            edges: &self.edges,
            visited_nodes: self.queue.len(),
            examined_edges,
        }
    }
}

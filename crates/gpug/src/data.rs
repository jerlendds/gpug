use std::collections::HashMap;
use std::fmt;

use crate::{Edge, Node, NodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayoutEdge {
    pub source: usize,
    pub target: usize,
}

#[derive(Clone, Debug, Default)]
pub struct GraphData {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphDataError {
    DuplicateNode(NodeId),
    UnknownEndpoint(NodeId),
}

impl fmt::Display for GraphDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(formatter, "duplicate node id {}", id.0),
            Self::UnknownEndpoint(id) => write!(formatter, "edge references unknown node {}", id.0),
        }
    }
}

impl std::error::Error for GraphDataError {}

impl GraphData {
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        Self { nodes, edges }
    }

    pub fn compile_edges(&self) -> Result<Vec<LayoutEdge>, GraphDataError> {
        let mut indices = HashMap::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if indices.insert(node.id, index).is_some() {
                return Err(GraphDataError::DuplicateNode(node.id));
            }
        }
        self.edges
            .iter()
            .map(|edge| {
                Ok(LayoutEdge {
                    source: *indices
                        .get(&edge.source)
                        .ok_or(GraphDataError::UnknownEndpoint(edge.source))?,
                    target: *indices
                        .get(&edge.target)
                        .ok_or(GraphDataError::UnknownEndpoint(edge.target))?,
                })
            })
            .collect()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldPoint;

    #[test]
    fn compiles_arbitrary_ids_to_dense_indices() {
        let data = GraphData::new(
            vec![
                Node::new(42u64, WorldPoint::ZERO),
                Node::new(7u64, WorldPoint::ZERO),
            ],
            vec![Edge::new(7u64, 42u64)],
        );
        let edges = data.compile_edges().unwrap();
        assert_eq!((edges[0].source, edges[0].target), (1, 0));
    }

    #[test]
    fn rejects_unknown_endpoints() {
        let data = GraphData::new(
            vec![Node::new(1u64, WorldPoint::ZERO)],
            vec![Edge::new(1u64, 2u64)],
        );
        assert_eq!(
            data.compile_edges(),
            Err(GraphDataError::UnknownEndpoint(NodeId(2)))
        );
    }
}

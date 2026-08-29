use std::collections::HashMap;
use std::fmt;

use crate::{Edge, EdgeId, Node, NodeId};

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
    DuplicateEdge(EdgeId),
    DuplicateConnection {
        source: NodeId,
        target: NodeId,
    },
    UnknownNode(NodeId),
    UnknownEdge(EdgeId),
    NodeReplacementIdMismatch {
        targeted: NodeId,
        replacement: NodeId,
    },
    EdgeReplacementIdMismatch {
        targeted: EdgeId,
        replacement: EdgeId,
    },
    UnknownParent(NodeId),
    ParentAfterChild {
        parent: NodeId,
        child: NodeId,
    },
    NonFiniteNodeGeometry(NodeId),
}

impl fmt::Display for GraphDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(formatter, "duplicate node id {}", id.0),
            Self::UnknownEndpoint(id) => write!(formatter, "edge references unknown node {}", id.0),
            Self::DuplicateEdge(id) => write!(formatter, "duplicate edge id {}", id.0),
            Self::DuplicateConnection { source, target } => write!(
                formatter,
                "duplicate connection from node {} to node {}",
                source.0, target.0
            ),
            Self::UnknownNode(id) => write!(formatter, "unknown node id {}", id.0),
            Self::UnknownEdge(id) => write!(formatter, "unknown edge id {}", id.0),
            Self::NodeReplacementIdMismatch {
                targeted,
                replacement,
            } => write!(
                formatter,
                "replacement node id {} differs from targeted id {}",
                replacement.0, targeted.0
            ),
            Self::EdgeReplacementIdMismatch {
                targeted,
                replacement,
            } => write!(
                formatter,
                "replacement edge id {} differs from targeted id {}",
                replacement.0, targeted.0
            ),
            Self::UnknownParent(id) => write!(formatter, "node references unknown parent {}", id.0),
            Self::ParentAfterChild { parent, child } => write!(
                formatter,
                "parent {} must precede child {}",
                parent.0, child.0
            ),
            Self::NonFiniteNodeGeometry(id) => {
                write!(formatter, "node {} contains non-finite geometry", id.0)
            }
        }
    }
}

impl std::error::Error for GraphDataError {}

impl GraphData {
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        Self { nodes, edges }
    }

    pub fn compile_edges(&self) -> Result<Vec<LayoutEdge>, GraphDataError> {
        compile_edges(&self.nodes, &self.edges)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }
}

/// Compiles stable node ids to dense topology indices without owning the data,
/// so hot paths can validate and index in place instead of cloning the graph.
pub fn compile_edges(nodes: &[Node], edges: &[Edge]) -> Result<Vec<LayoutEdge>, GraphDataError> {
    let mut indices = HashMap::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        if indices.insert(node.id, index).is_some() {
            return Err(GraphDataError::DuplicateNode(node.id));
        }
    }
    for (index, node) in nodes.iter().enumerate() {
        let extent_is_finite = node.extent.is_none_or(|extent| {
            extent.origin.x.is_finite()
                && extent.origin.y.is_finite()
                && extent.size.width.is_finite()
                && extent.size.height.is_finite()
        });
        if !node.position.x.is_finite()
            || !node.position.y.is_finite()
            || !node.size.width.is_finite()
            || !node.size.height.is_finite()
            || !node.origin.x.is_finite()
            || !node.origin.y.is_finite()
            || !extent_is_finite
        {
            return Err(GraphDataError::NonFiniteNodeGeometry(node.id));
        }
        if let Some(parent) = node.parent_id {
            match indices.get(&parent) {
                Some(parent_index) if *parent_index < index => {}
                Some(_) => {
                    return Err(GraphDataError::ParentAfterChild {
                        parent,
                        child: node.id,
                    });
                }
                None => return Err(GraphDataError::UnknownParent(parent)),
            }
        }
    }
    let mut edge_ids = std::collections::HashSet::with_capacity(edges.len());
    for edge in edges {
        if !edge_ids.insert(edge.id) {
            return Err(GraphDataError::DuplicateEdge(edge.id));
        }
    }
    let layout_edges = edges
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
        .collect::<Result<Vec<_>, _>>()?;
    let mut connections = std::collections::HashSet::with_capacity(edges.len());
    for edge in edges {
        if !connections.insert((
            edge.source,
            edge.target,
            edge.source_handle.as_deref(),
            edge.target_handle.as_deref(),
        )) {
            return Err(GraphDataError::DuplicateConnection {
                source: edge.source,
                target: edge.target,
            });
        }
    }
    Ok(layout_edges)
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

    #[test]
    fn rejects_child_before_parent() {
        let data = GraphData::new(
            vec![
                Node::new(2u64, WorldPoint::ZERO).with_parent(1u64),
                Node::new(1u64, WorldPoint::ZERO),
            ],
            vec![],
        );
        assert_eq!(
            data.compile_edges(),
            Err(GraphDataError::ParentAfterChild {
                parent: NodeId(1),
                child: NodeId(2)
            })
        );
    }

    #[test]
    fn rejects_duplicate_edge_ids_and_unknown_parents() {
        let nodes = vec![Node::new(1u64, WorldPoint::ZERO)];
        let duplicate = GraphData::new(
            nodes.clone(),
            vec![
                Edge::new(1u64, 1u64).with_id(4u64),
                Edge::new(1u64, 1u64).with_id(4u64),
            ],
        );
        assert_eq!(
            duplicate.compile_edges(),
            Err(GraphDataError::DuplicateEdge(EdgeId(4)))
        );

        let unknown_parent = GraphData::new(
            vec![Node::new(1u64, WorldPoint::ZERO).with_parent(2u64)],
            vec![],
        );
        assert_eq!(
            unknown_parent.compile_edges(),
            Err(GraphDataError::UnknownParent(NodeId(2)))
        );
    }

    #[test]
    fn rejects_duplicate_connections() {
        let data = GraphData::new(
            vec![
                Node::new(1u64, WorldPoint::ZERO),
                Node::new(2u64, WorldPoint::ZERO),
            ],
            vec![
                Edge::new(1u64, 2u64).with_id(1u64),
                Edge::new(1u64, 2u64).with_id(2u64),
            ],
        );

        assert_eq!(
            data.compile_edges(),
            Err(GraphDataError::DuplicateConnection {
                source: NodeId(1),
                target: NodeId(2),
            })
        );
    }

    #[test]
    fn rejects_non_finite_node_geometry() {
        let data = GraphData::new(
            vec![Node::new(1u64, WorldPoint::new(f32::NAN, 0.0))],
            vec![],
        );
        assert_eq!(
            data.compile_edges(),
            Err(GraphDataError::NonFiniteNodeGeometry(NodeId(1)))
        );
    }
}

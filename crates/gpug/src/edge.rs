use crate::node::NodeId;

#[derive(Clone, Debug)]
pub struct Edge {
    pub source: NodeId,
    pub target: NodeId,
}

impl Edge {
    pub fn new(source: impl Into<NodeId>, target: impl Into<NodeId>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
        }
    }
}

#[deprecated(note = "use Edge")]
pub type GpugEdge = Edge;

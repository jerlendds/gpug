use crate::coordinates::{WorldPoint, WorldSize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(pub u64);

impl NodeId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl From<u64> for NodeId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<usize> for NodeId {
    fn from(value: usize) -> Self {
        Self(value as u64)
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub position: WorldPoint,
    pub size: WorldSize,
    pub selected: bool,
}

impl Node {
    pub fn new(id: impl Into<NodeId>, position: WorldPoint) -> Self {
        Self {
            id: id.into(),
            position,
            size: WorldSize::new(4.0, 4.0),
            selected: false,
        }
    }

    pub fn with_size(mut self, size: WorldSize) -> Self {
        self.size = size;
        self
    }
}

#[deprecated(note = "use Node")]
pub type GpugNode = Node;

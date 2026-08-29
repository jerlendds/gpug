use crate::coordinates::{WorldPoint, WorldSize};
use crate::WorldBounds;
use std::collections::HashMap;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub position: WorldPoint,
    pub size: WorldSize,
    pub selected: bool,
    pub node_type: String,
    pub parent_id: Option<NodeId>,
    pub draggable: bool,
    pub selectable: bool,
    pub connectable: bool,
    pub deletable: bool,
    pub focusable: bool,
    pub origin: WorldPoint,
    pub extent: Option<WorldBounds>,
    pub expand_parent: bool,
    pub metadata: HashMap<String, String>,
    pub hidden: bool,
}

impl Node {
    pub fn new(id: impl Into<NodeId>, position: WorldPoint) -> Self {
        Self {
            id: id.into(),
            position,
            size: WorldSize::new(4.0, 4.0),
            selected: false,
            node_type: "default".into(),
            parent_id: None,
            draggable: true,
            selectable: true,
            connectable: true,
            deletable: true,
            focusable: true,
            origin: WorldPoint::new(0.5, 0.5),
            extent: None,
            expand_parent: false,
            metadata: HashMap::new(),
            hidden: false,
        }
    }

    pub fn with_size(mut self, size: WorldSize) -> Self {
        self.size = size;
        self
    }
    pub fn with_type(mut self, node_type: impl Into<String>) -> Self {
        self.node_type = node_type.into();
        self
    }
    pub fn with_parent(mut self, parent_id: impl Into<NodeId>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }
    pub fn with_extent(mut self, extent: WorldBounds) -> Self {
        self.extent = Some(extent);
        self
    }
    pub fn with_origin(mut self, origin: WorldPoint) -> Self {
        self.origin = origin;
        self
    }
}

#[deprecated(note = "use Node")]
pub type GpugNode = Node;

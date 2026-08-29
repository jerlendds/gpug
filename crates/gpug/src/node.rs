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
    /// Treats the node body as an invisible source/target handle.
    ///
    /// When this is enabled, use `custom_handle` to reserve a region that can
    /// still initiate node dragging.
    pub connectable_body: bool,
    pub deletable: bool,
    pub focusable: bool,
    pub origin: WorldPoint,
    pub extent: Option<WorldBounds>,
    pub expand_parent: bool,
    pub metadata: HashMap<String, String>,
    pub hidden: bool,
    /// Optional node-local rectangle that is allowed to initiate dragging.
    /// Coordinates are measured from the node's top-left corner in world units.
    pub custom_handle: Option<WorldBounds>,
    /// Node-local rectangles that never initiate dragging. These take
    /// precedence over `custom_handle`, which permits interactive children
    /// inside a larger drag handle.
    pub nodrag: Vec<WorldBounds>,
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
            connectable_body: false,
            deletable: true,
            focusable: true,
            origin: WorldPoint::new(0.5, 0.5),
            extent: None,
            expand_parent: false,
            metadata: HashMap::new(),
            hidden: false,
            custom_handle: None,
            nodrag: Vec::new(),
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

    /// Makes the complete node body participate in connection hit testing.
    pub fn with_connectable_body(mut self) -> Self {
        self.connectable_body = true;
        self
    }

    /// Restricts pointer dragging to `bounds`, expressed in node-local world
    /// coordinates from the node's top-left corner.
    pub fn with_custom_handle(mut self, bounds: WorldBounds) -> Self {
        self.custom_handle = Some(bounds);
        self
    }

    /// Prevents pointer dragging from a node-local rectangle. No-drag regions
    /// override both whole-node dragging and a custom drag handle.
    pub fn with_nodrag(mut self, bounds: WorldBounds) -> Self {
        self.nodrag.push(bounds);
        self
    }

    /// Returns whether a pointer at `local` may start a node drag.
    pub fn allows_drag_at(&self, local: WorldPoint) -> bool {
        let contains = |bounds: &WorldBounds| {
            local.x >= bounds.origin.x
                && local.y >= bounds.origin.y
                && local.x <= bounds.origin.x + bounds.size.width
                && local.y <= bounds.origin.y + bounds.size.height
        };
        self.draggable
            && !self.nodrag.iter().any(contains)
            && self.custom_handle.as_ref().is_none_or(contains)
    }
}

#[deprecated(note = "use Node")]
pub type GpugNode = Node;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_handle_allows_only_its_region() {
        let node = Node::new(1_u64, WorldPoint::ZERO).with_custom_handle(WorldBounds::new(
            WorldPoint::new(7.0, 1.0),
            WorldSize::new(2.0, 2.0),
        ));
        assert!(node.allows_drag_at(WorldPoint::new(8.0, 2.0)));
        assert!(!node.allows_drag_at(WorldPoint::new(4.0, 2.0)));
    }

    #[test]
    fn nodrag_overrides_a_custom_handle() {
        let node = Node::new(1_u64, WorldPoint::ZERO)
            .with_custom_handle(WorldBounds::new(
                WorldPoint::new(7.0, 1.0),
                WorldSize::new(2.0, 2.0),
            ))
            .with_nodrag(WorldBounds::new(
                WorldPoint::new(7.5, 1.5),
                WorldSize::new(1.0, 1.0),
            ));
        assert!(node.allows_drag_at(WorldPoint::new(7.25, 1.25)));
        assert!(!node.allows_drag_at(WorldPoint::new(8.0, 2.0)));
    }
}

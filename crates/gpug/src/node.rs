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
    /// Directions in which resize gestures are accepted. `None` enables all
    /// eight directions when graph resize handles are enabled.
    pub resize_directions: Option<Vec<crate::ResizeDirection>>,
    /// Whether the graph paints its standard resize handles for this node.
    /// Hit testing remains active so custom node content can provide the UI.
    pub show_resize_controls: bool,
    /// Half-size of each resize control's square hit region in screen pixels.
    pub resize_control_hit_radius: f32,
    /// Keeps resize controls interactive and visible without node selection.
    pub resize_controls_always_visible: bool,
    /// Optional color override for this node's resize controls.
    pub resize_control_color: Option<u32>,
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
            resize_directions: None,
            show_resize_controls: true,
            resize_control_hit_radius: 8.0,
            resize_controls_always_visible: false,
            resize_control_color: None,
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

    /// Restricts resize gestures to the supplied directions.
    pub fn with_resize_directions(
        mut self,
        directions: impl IntoIterator<Item = crate::ResizeDirection>,
    ) -> Self {
        self.resize_directions = Some(directions.into_iter().collect());
        self
    }

    /// Hides standard resize handles while retaining their hit regions for a
    /// custom resize control rendered as part of the node.
    pub fn with_custom_resize_controls(mut self) -> Self {
        self.show_resize_controls = false;
        self
    }

    /// Sets the screen-space hit radius for custom resize controls.
    pub fn with_resize_control_hit_radius(mut self, radius_pixels: f32) -> Self {
        if radius_pixels.is_finite() && radius_pixels >= 0.0 {
            self.resize_control_hit_radius = radius_pixels;
        }
        self
    }

    /// Keeps this node's resize controls visible and interactive at all times.
    pub fn with_always_visible_resize_controls(mut self) -> Self {
        self.resize_controls_always_visible = true;
        self
    }

    /// Overrides the graph selection color for this node's resize controls.
    pub fn with_resize_control_color(mut self, color: u32) -> Self {
        self.resize_control_color = Some(color);
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

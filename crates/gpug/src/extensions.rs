//! Geometry and contracts for add-ons built entirely on public editor state.
use crate::{Node, NodeId, NodeRuntime, Position, Viewport, WorldBounds, WorldPoint, WorldSize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundPattern {
    Dots,
    Lines,
    Cross,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub pattern: BackgroundPattern,
    pub gap: f32,
    pub size: f32,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            pattern: BackgroundPattern::Dots,
            gap: 20.0,
            size: 1.0,
        }
    }
}
impl Background {
    pub fn offset(&self, viewport: Viewport) -> WorldPoint {
        let pan = viewport.pan();
        WorldPoint::new(
            (pan.x / gpui::px(1.0)).rem_euclid(self.gap * viewport.zoom()),
            (pan.y / gpui::px(1.0)).rem_euclid(self.gap * viewport.zoom()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniMapNode {
    pub id: NodeId,
    pub bounds: WorldBounds,
    pub selected: bool,
}
pub fn minimap_nodes(
    nodes: &[Node],
    runtimes: &HashMap<NodeId, NodeRuntime>,
    world: WorldBounds,
    target: WorldSize,
) -> Vec<MiniMapNode> {
    let scale = (target.width / world.size.width.max(f32::EPSILON))
        .min(target.height / world.size.height.max(f32::EPSILON));
    nodes
        .iter()
        .filter_map(|node| {
            let bounds = runtimes.get(&node.id)?.bounds();
            Some(MiniMapNode {
                id: node.id,
                bounds: WorldBounds::new(
                    WorldPoint::new(
                        (bounds.origin.x - world.origin.x) * scale,
                        (bounds.origin.y - world.origin.y) * scale,
                    ),
                    WorldSize::new(bounds.size.width * scale, bounds.size.height * scale),
                ),
                selected: node.selected,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct BoundsOptions {
    pub node_ids: Option<std::collections::HashSet<NodeId>>,
    pub include_hidden: bool,
}
pub fn get_nodes_bounds(
    nodes: &[Node],
    runtimes: &HashMap<NodeId, NodeRuntime>,
    options: &BoundsOptions,
) -> Option<WorldBounds> {
    let mut selected = nodes
        .iter()
        .filter(|node| {
            (options.include_hidden || !node.hidden)
                && options
                    .node_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&node.id))
        })
        .filter_map(|node| runtimes.get(&node.id).map(NodeRuntime::bounds));
    let first = selected.next()?;
    let (mut left, mut top, mut right, mut bottom) = (
        first.origin.x,
        first.origin.y,
        first.origin.x + first.size.width,
        first.origin.y + first.size.height,
    );
    for bounds in selected {
        left = left.min(bounds.origin.x);
        top = top.min(bounds.origin.y);
        right = right.max(bounds.origin.x + bounds.size.width);
        bottom = bottom.max(bounds.origin.y + bounds.size.height)
    }
    Some(WorldBounds::new(
        WorldPoint::new(left, top),
        WorldSize::new(right - left, bottom - top),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayLayer {
    EdgeLabels,
    Viewport,
    Toolbars,
    Panels,
    Menus,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarPlacement {
    pub anchor: WorldPoint,
    pub side: Position,
}

pub trait ViewportCommands {
    fn zoom_in(&mut self);
    fn zoom_out(&mut self);
    fn fit_view(&mut self);
    fn toggle_interactive(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node;
    #[test]
    fn minimap_uses_runtime_bounds() {
        let nodes = vec![Node::new(1u64, WorldPoint::new(50.0, 50.0))];
        let mut runtimes = HashMap::new();
        runtimes.insert(NodeId(1), NodeRuntime::from_node(&nodes[0]));
        let mapped = minimap_nodes(
            &nodes,
            &runtimes,
            WorldBounds::new(WorldPoint::ZERO, WorldSize::new(100.0, 100.0)),
            WorldSize::new(200.0, 100.0),
        );
        assert_eq!(mapped.len(), 1);
    }
    #[test]
    fn bounds_can_filter_hidden_nodes() {
        let mut hidden = Node::new(2u64, WorldPoint::new(100.0, 100.0));
        hidden.hidden = true;
        let nodes = vec![Node::new(1u64, WorldPoint::ZERO), hidden];
        let mut runtimes = HashMap::new();
        for node in &nodes {
            runtimes.insert(node.id, NodeRuntime::from_node(node));
        }
        let bounds = get_nodes_bounds(&nodes, &runtimes, &BoundsOptions::default()).unwrap();
        assert!(bounds.origin.x < 100.0);
    }
}

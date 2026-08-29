//! Pure node resize and overlay placement geometry.
use crate::{NodeId, Position, WorldBounds, WorldPoint, WorldSize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeDirection {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}
#[derive(Clone, Copy, Debug)]
pub struct ResizeOptions {
    pub min: WorldSize,
    pub max: WorldSize,
    pub keep_aspect: bool,
    pub snap: Option<WorldSize>,
    pub extent: Option<WorldBounds>,
}
impl Default for ResizeOptions {
    fn default() -> Self {
        Self {
            min: WorldSize::new(1.0, 1.0),
            max: WorldSize::new(f32::MAX, f32::MAX),
            keep_aspect: false,
            snap: None,
            extent: None,
        }
    }
}

/// Stateful resize gesture for building custom node resize UI.
///
/// A control is deliberately independent of GPUI elements: applications can
/// attach it to any knob, border, or icon and feed it world-space pointer
/// positions. [`Graph`](crate::Graph) provides `begin_node_resize`,
/// `update_node_resize`, and `end_node_resize` helpers which apply its output.
#[derive(Clone, Copy, Debug)]
pub struct NodeResizeControl {
    node_id: NodeId,
    direction: ResizeDirection,
    options: ResizeOptions,
    gesture: Option<(WorldPoint, WorldBounds)>,
}

impl NodeResizeControl {
    pub fn new(node_id: impl Into<NodeId>, direction: ResizeDirection) -> Self {
        Self {
            node_id: node_id.into(),
            direction,
            options: ResizeOptions::default(),
            gesture: None,
        }
    }

    pub fn with_options(mut self, options: ResizeOptions) -> Self {
        self.options = options;
        self
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn direction(&self) -> ResizeDirection {
        self.direction
    }

    pub fn options(&self) -> ResizeOptions {
        self.options
    }

    pub fn is_resizing(&self) -> bool {
        self.gesture.is_some()
    }

    pub fn begin(&mut self, pointer: WorldPoint, bounds: WorldBounds) {
        self.gesture = Some((pointer, bounds));
    }

    pub fn update(&self, pointer: WorldPoint) -> Option<WorldBounds> {
        let (start, bounds) = self.gesture?;
        Some(resize_bounds(
            bounds,
            self.direction,
            WorldPoint::new(pointer.x - start.x, pointer.y - start.y),
            self.options,
        ))
    }

    pub fn end(&mut self, pointer: WorldPoint) -> Option<WorldBounds> {
        let bounds = self.update(pointer);
        self.gesture = None;
        bounds
    }

    pub fn cancel(&mut self) {
        self.gesture = None;
    }
}

pub fn resize_bounds(
    bounds: WorldBounds,
    direction: ResizeDirection,
    delta: WorldPoint,
    options: ResizeOptions,
) -> WorldBounds {
    let west = matches!(
        direction,
        ResizeDirection::West | ResizeDirection::NorthWest | ResizeDirection::SouthWest
    );
    let east = matches!(
        direction,
        ResizeDirection::East | ResizeDirection::NorthEast | ResizeDirection::SouthEast
    );
    let north = matches!(
        direction,
        ResizeDirection::North | ResizeDirection::NorthEast | ResizeDirection::NorthWest
    );
    let south = matches!(
        direction,
        ResizeDirection::South | ResizeDirection::SouthEast | ResizeDirection::SouthWest
    );
    let mut left = bounds.origin.x;
    let mut top = bounds.origin.y;
    let mut right = left + bounds.size.width;
    let mut bottom = top + bounds.size.height;
    if west {
        left += delta.x
    }
    if east {
        right += delta.x
    }
    if north {
        top += delta.y
    }
    if south {
        bottom += delta.y
    }
    let aspect = bounds.size.width / bounds.size.height.max(f32::EPSILON);
    let mut width = (right - left).clamp(options.min.width, options.max.width);
    let mut height = (bottom - top).clamp(options.min.height, options.max.height);
    if options.keep_aspect {
        if delta.x.abs() >= delta.y.abs() {
            height = width / aspect
        } else {
            width = height * aspect
        }
    }
    if let Some(grid) = options.snap {
        if grid.width > 0.0 {
            width = (width / grid.width).round() * grid.width
        }
        if grid.height > 0.0 {
            height = (height / grid.height).round() * grid.height
        }
    }
    if west {
        left = right - width
    } else {
        right = left + width
    }
    if north {
        top = bottom - height
    } else {
        bottom = top + height
    }
    if let Some(extent) = options.extent {
        left = left.max(extent.origin.x);
        top = top.max(extent.origin.y);
        right = right.min(extent.origin.x + extent.size.width);
        bottom = bottom.min(extent.origin.y + extent.size.height)
    }
    WorldBounds::new(
        WorldPoint::new(left, top),
        WorldSize::new((right - left).max(0.0), (bottom - top).max(0.0)),
    )
}
pub fn toolbar_position(bounds: WorldBounds, side: Position, offset: f32) -> WorldPoint {
    match side {
        Position::Top => WorldPoint::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y - offset,
        ),
        Position::Bottom => WorldPoint::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y + bounds.size.height + offset,
        ),
        Position::Left => WorldPoint::new(
            bounds.origin.x - offset,
            bounds.origin.y + bounds.size.height * 0.5,
        ),
        Position::Right => WorldPoint::new(
            bounds.origin.x + bounds.size.width + offset,
            bounds.origin.y + bounds.size.height * 0.5,
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn east_resize_respects_limits() {
        let result = resize_bounds(
            WorldBounds::new(WorldPoint::ZERO, WorldSize::new(10.0, 10.0)),
            ResizeDirection::East,
            WorldPoint::new(20.0, 0.0),
            ResizeOptions {
                max: WorldSize::new(15.0, 15.0),
                ..Default::default()
            },
        );
        assert_eq!(result.size.width, 15.0)
    }

    #[test]
    fn control_preserves_the_opposite_corner() {
        let mut control = NodeResizeControl::new(7u64, ResizeDirection::NorthWest);
        control.begin(
            WorldPoint::new(10.0, 10.0),
            WorldBounds::new(WorldPoint::new(10.0, 10.0), WorldSize::new(20.0, 20.0)),
        );
        let resized = control.update(WorldPoint::new(5.0, 6.0)).unwrap();
        assert_eq!(resized.origin, WorldPoint::new(5.0, 6.0));
        assert_eq!(resized.size, WorldSize::new(25.0, 24.0));
        assert!(control.is_resizing());
        control.end(WorldPoint::new(5.0, 6.0));
        assert!(!control.is_resizing());
    }
}

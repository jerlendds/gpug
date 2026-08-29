//! Pure node resize and overlay placement geometry.
use crate::{Position, WorldBounds, WorldPoint, WorldSize};

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
}

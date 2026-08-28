use gpui::{point, px, Pixels, Point};

/// A position in GPUG's renderer-independent graph world.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldPoint {
    pub x: f32,
    pub y: f32,
}

impl WorldPoint {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size expressed in world units. It scales with the viewport zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldSize {
    pub width: f32,
    pub height: f32,
}

impl WorldSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Double-precision coordinate used at an external layout boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutPoint {
    pub x: f64,
    pub y: f64,
}

impl From<WorldPoint> for LayoutPoint {
    fn from(value: WorldPoint) -> Self {
        Self {
            x: value.x as f64,
            y: value.y as f64,
        }
    }
}

impl From<LayoutPoint> for WorldPoint {
    fn from(value: LayoutPoint) -> Self {
        Self::new(value.x as f32, value.y as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBounds {
    pub origin: WorldPoint,
    pub size: WorldSize,
}

impl WorldBounds {
    pub const fn new(origin: WorldPoint, size: WorldSize) -> Self {
        Self { origin, size }
    }
}

/// The only bridge between graph world coordinates and GPUI screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pan: Point<Pixels>,
    zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new(point(px(0.0), px(0.0)), 1.0)
    }
}

impl Viewport {
    /// Smallest supported scale: one screen pixel represents 1,000 world units.
    pub const MIN_ZOOM: f32 = 0.001;
    /// Largest supported scale: one world unit represents 256 screen pixels.
    pub const MAX_ZOOM: f32 = 256.0;

    pub fn new(pan: Point<Pixels>, zoom: f32) -> Self {
        Self {
            pan,
            zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM),
        }
    }

    pub fn pan(&self) -> Point<Pixels> {
        self.pan
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn set_pan(&mut self, pan: Point<Pixels>) {
        self.pan = pan;
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
    }

    pub fn world_to_screen(&self, world: WorldPoint) -> Point<Pixels> {
        point(
            self.pan.x + px(world.x * self.zoom),
            self.pan.y + px(world.y * self.zoom),
        )
    }

    pub fn screen_to_world(&self, screen: Point<Pixels>) -> WorldPoint {
        WorldPoint::new(
            ((screen.x - self.pan.x) / px(1.0)) / self.zoom,
            ((screen.y - self.pan.y) / px(1.0)) / self.zoom,
        )
    }

    pub fn zoom_about(&mut self, screen_anchor: Point<Pixels>, new_zoom: f32) {
        let world_anchor = self.screen_to_world(screen_anchor);
        self.set_zoom(new_zoom);
        self.pan = point(
            screen_anchor.x - px(world_anchor.x * self.zoom),
            screen_anchor.y - px(world_anchor.y * self.zoom),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_round_trip_is_stable() {
        let viewport = Viewport::new(point(px(81.0), px(-27.0)), 2.5);
        let world = WorldPoint::new(123.25, -44.5);
        let result = viewport.screen_to_world(viewport.world_to_screen(world));
        assert!((result.x - world.x).abs() < 0.0001);
        assert!((result.y - world.y).abs() < 0.0001);
    }

    #[test]
    fn zoom_about_preserves_anchor() {
        let mut viewport = Viewport::default();
        let anchor = point(px(300.0), px(200.0));
        let world = viewport.screen_to_world(anchor);
        viewport.zoom_about(anchor, 3.0);
        assert_eq!(viewport.screen_to_world(anchor), world);
    }

    #[test]
    fn zoom_is_clamped_to_supported_range() {
        let mut viewport = Viewport::default();
        viewport.set_zoom(0.0);
        assert_eq!(viewport.zoom(), Viewport::MIN_ZOOM);
        viewport.set_zoom(f32::MAX);
        assert_eq!(viewport.zoom(), Viewport::MAX_ZOOM);
    }
}

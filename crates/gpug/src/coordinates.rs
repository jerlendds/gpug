use gpui::{Pixels, Point, point, px};

/// A position in GPUG's renderer-independent graph world.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldPoint {
    pub x: f32,
    pub y: f32,
}

/// Pointer position relative to the application window, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenPoint {
    pub x: f32,
    pub y: f32,
}

impl ScreenPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Pointer position relative to the graph pane, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportPoint {
    pub x: f32,
    pub y: f32,
}

impl ViewportPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
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
        let pan = if (pan.x / px(1.0)).is_finite() && (pan.y / px(1.0)).is_finite() {
            pan
        } else {
            point(px(0.0), px(0.0))
        };
        Self {
            pan,
            zoom: if zoom.is_finite() {
                zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM)
            } else {
                1.0
            },
        }
    }

    pub fn pan(&self) -> Point<Pixels> {
        self.pan
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn set_pan(&mut self, pan: Point<Pixels>) {
        if (pan.x / px(1.0)).is_finite() && (pan.y / px(1.0)).is_finite() {
            self.pan = pan;
        }
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        }
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

    pub fn viewport_to_world(&self, point: ViewportPoint, snap: Option<WorldSize>) -> WorldPoint {
        let mut result = WorldPoint::new(
            (point.x - self.pan.x / px(1.0)) / self.zoom,
            (point.y - self.pan.y / px(1.0)) / self.zoom,
        );
        if let Some(grid) = snap {
            if grid.width > 0.0 {
                result.x = (result.x / grid.width).round() * grid.width;
            }
            if grid.height > 0.0 {
                result.y = (result.y / grid.height).round() * grid.height;
            }
        }
        result
    }

    pub fn world_to_viewport(&self, point: WorldPoint) -> ViewportPoint {
        ViewportPoint::new(
            self.pan.x / px(1.0) + point.x * self.zoom,
            self.pan.y / px(1.0) + point.y * self.zoom,
        )
    }

    pub fn zoom_about(&mut self, screen_anchor: Point<Pixels>, new_zoom: f32) {
        if !(screen_anchor.x / px(1.0)).is_finite()
            || !(screen_anchor.y / px(1.0)).is_finite()
            || !new_zoom.is_finite()
        {
            return;
        }
        let world_anchor = self.screen_to_world(screen_anchor);
        self.set_zoom(new_zoom);
        self.pan = point(
            screen_anchor.x - px(world_anchor.x * self.zoom),
            screen_anchor.y - px(world_anchor.y * self.zoom),
        );
    }

    pub fn pan_by(&mut self, delta: ViewportPoint) {
        if delta.x.is_finite() && delta.y.is_finite() {
            self.pan = point(self.pan.x + px(delta.x), self.pan.y + px(delta.y));
        }
    }

    pub fn set_center(&mut self, world: WorldPoint, pane_size: WorldSize, zoom: f32) {
        if !world.x.is_finite()
            || !world.y.is_finite()
            || !pane_size.width.is_finite()
            || !pane_size.height.is_finite()
            || !zoom.is_finite()
        {
            return;
        }
        self.set_zoom(zoom);
        self.pan = point(
            px(pane_size.width * 0.5 - world.x * self.zoom),
            px(pane_size.height * 0.5 - world.y * self.zoom),
        );
    }

    pub fn fit_bounds(&mut self, bounds: WorldBounds, pane_size: WorldSize, padding: f32) {
        if !bounds.origin.x.is_finite()
            || !bounds.origin.y.is_finite()
            || !bounds.size.width.is_finite()
            || !bounds.size.height.is_finite()
            || !pane_size.width.is_finite()
            || !pane_size.height.is_finite()
            || !padding.is_finite()
        {
            return;
        }
        let available_width = (pane_size.width - padding * 2.0).max(1.0);
        let available_height = (pane_size.height - padding * 2.0).max(1.0);
        let zoom = (available_width / bounds.size.width.max(0.0001))
            .min(available_height / bounds.size.height.max(0.0001));
        let center = WorldPoint::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y + bounds.size.height * 0.5,
        );
        self.set_center(center, pane_size, zoom);
    }

    pub fn constrain(&mut self, extent: WorldBounds, pane_size: WorldSize) {
        if !extent.origin.x.is_finite()
            || !extent.origin.y.is_finite()
            || !extent.size.width.is_finite()
            || !extent.size.height.is_finite()
            || !pane_size.width.is_finite()
            || !pane_size.height.is_finite()
        {
            return;
        }
        let min_x = pane_size.width - (extent.origin.x + extent.size.width) * self.zoom;
        let max_x = -extent.origin.x * self.zoom;
        let min_y = pane_size.height - (extent.origin.y + extent.size.height) * self.zoom;
        let max_y = -extent.origin.y * self.zoom;
        let x = self.pan.x / px(1.0);
        let y = self.pan.y / px(1.0);
        self.pan = point(
            px(if min_x <= max_x {
                x.clamp(min_x, max_x)
            } else {
                (min_x + max_x) * 0.5
            }),
            px(if min_y <= max_y {
                y.clamp(min_y, max_y)
            } else {
                (min_y + max_y) * 0.5
            }),
        );
    }

    /// Pan delta for a pointer inside the pane's edge zone; zero elsewhere.
    /// Kept separate from [`Viewport::auto_pan`] so a caller can ask whether an
    /// auto-pan is still live without moving the camera to find out.
    pub fn auto_pan_delta(
        &self,
        pointer: ViewportPoint,
        pane_size: WorldSize,
        margin: f32,
        speed: f32,
    ) -> ViewportPoint {
        if !pointer.x.is_finite()
            || !pointer.y.is_finite()
            || !pane_size.width.is_finite()
            || !pane_size.height.is_finite()
            || !margin.is_finite()
            || !speed.is_finite()
        {
            return ViewportPoint::new(0.0, 0.0);
        }
        fn axis(value: f32, limit: f32, margin: f32, speed: f32) -> f32 {
            if margin <= 0.0 {
                return 0.0;
            }
            if value < margin {
                speed * ((margin - value) / margin).clamp(0.0, 1.0)
            } else if value > limit - margin {
                -speed * ((value - (limit - margin)) / margin).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
        ViewportPoint::new(
            axis(pointer.x, pane_size.width, margin, speed),
            axis(pointer.y, pane_size.height, margin, speed),
        )
    }

    /// Returns and applies a pan delta when a pointer enters the pane's edge zone.
    pub fn auto_pan(
        &mut self,
        pointer: ViewportPoint,
        pane_size: WorldSize,
        margin: f32,
        speed: f32,
    ) -> ViewportPoint {
        let delta = self.auto_pan_delta(pointer, pane_size, margin, speed);
        self.pan_by(delta);
        delta
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

    #[test]
    fn non_finite_viewport_inputs_do_not_poison_transforms() {
        let mut viewport = Viewport::new(point(px(f32::NAN), px(2.0)), f32::NAN);
        assert_eq!(viewport.pan(), point(px(0.0), px(0.0)));
        assert_eq!(viewport.zoom(), 1.0);
        viewport.set_zoom(2.0);
        viewport.set_zoom(f32::INFINITY);
        viewport.set_pan(point(px(f32::NAN), px(4.0)));
        assert_eq!(viewport.zoom(), 2.0);
        assert_eq!(viewport.pan(), point(px(0.0), px(0.0)));
        viewport.fit_bounds(
            WorldBounds::new(WorldPoint::ZERO, WorldSize::new(f32::NAN, 10.0)),
            WorldSize::new(100.0, 100.0),
            0.0,
        );
        assert_eq!(viewport.zoom(), 2.0);
    }

    #[test]
    fn non_finite_constraints_and_auto_pan_are_ignored() {
        let mut viewport = Viewport::new(point(px(4.0), px(8.0)), 2.0);
        viewport.constrain(
            WorldBounds::new(WorldPoint::new(f32::NAN, 0.0), WorldSize::new(10.0, 10.0)),
            WorldSize::new(100.0, 100.0),
        );
        assert_eq!(viewport.pan(), point(px(4.0), px(8.0)));
        assert_eq!(
            viewport.auto_pan_delta(
                ViewportPoint::new(f32::INFINITY, 5.0),
                WorldSize::new(100.0, 100.0),
                20.0,
                5.0,
            ),
            ViewportPoint::new(0.0, 0.0)
        );
    }

    #[test]
    fn fit_bounds_centers_content() {
        let mut viewport = Viewport::default();
        viewport.fit_bounds(
            WorldBounds::new(WorldPoint::new(0.0, 0.0), WorldSize::new(100.0, 50.0)),
            WorldSize::new(200.0, 200.0),
            0.0,
        );
        assert_eq!(viewport.zoom(), 2.0);
        assert_eq!(
            viewport.world_to_viewport(WorldPoint::new(50.0, 25.0)),
            ViewportPoint::new(100.0, 100.0)
        );
    }
}

use super::*;

impl Graph {
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.smooth_zoom = None;
        self.events.push(GraphEvent::ViewportChanged(viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn sync_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.smooth_zoom = None;
        self.model.store.dirty.mark_viewport();
    }

    pub fn set_pan(&mut self, pan: Point<Pixels>) {
        self.viewport.set_pan(pan);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }

    pub fn pan_by(&mut self, delta: Point<Pixels>) {
        let pan = self.viewport.pan();
        self.viewport
            .set_pan(point(pan.x + delta.x, pan.y + delta.y));
        self.model.store.dirty.mark_viewport();
    }

    pub fn zoom_in(&mut self, anchor: Point<Pixels>) {
        self.viewport
            .zoom_about(self.local_screen(anchor), self.viewport.zoom() * 1.2);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn zoom_out(&mut self, anchor: Point<Pixels>) {
        self.viewport
            .zoom_about(self.local_screen(anchor), self.viewport.zoom() / 1.2);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    /// World-space bounds of every visible node, or `None` when the graph is
    /// empty. This is what `fit_to_view` fits, exposed so a caller can drive
    /// the camera itself without reaching into the store.
    pub fn content_bounds(&self) -> Option<WorldBounds> {
        world_bounds(&self.model.nodes, &self.model.store)
    }

    pub fn set_center(&mut self, world: WorldPoint, screen_size: Size<Pixels>, zoom: f32) {
        self.viewport.set_center(
            world,
            crate::WorldSize::new(screen_size.width / px(1.0), screen_size.height / px(1.0)),
            zoom,
        );
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn fit_bounds(&mut self, bounds: WorldBounds, screen_size: Size<Pixels>, padding: Pixels) {
        self.viewport.fit_bounds(
            bounds,
            crate::WorldSize::new(screen_size.width / px(1.0), screen_size.height / px(1.0)),
            padding / px(1.0),
        );
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn screen_to_flow_position(
        &self,
        point: Point<Pixels>,
        snap: Option<crate::WorldSize>,
    ) -> WorldPoint {
        let origin = self.canvas_origin();
        self.viewport.viewport_to_world(
            ViewportPoint::new(
                (point.x - origin.x) / px(1.0),
                (point.y - origin.y) / px(1.0),
            ),
            snap,
        )
    }
    pub fn flow_to_screen_position(&self, point: WorldPoint) -> Point<Pixels> {
        self.world_to_screen(point)
    }

    pub fn world_to_screen(&self, point: WorldPoint) -> Point<Pixels> {
        let local = self.viewport.world_to_screen(point);
        local_to_window(local, self.canvas_origin())
    }

    /// Converts GPUI window coordinates to world coordinates.
    ///
    /// Pass `MouseEvent::position` directly; GPUG removes the graph's laid-out
    /// component origin before applying its local viewport transform.
    pub fn screen_to_world(&self, point: Point<Pixels>) -> WorldPoint {
        self.viewport
            .screen_to_world(window_to_local(point, self.canvas_origin()))
    }

    pub(super) fn canvas_origin(&self) -> Point<Pixels> {
        self.canvas_bounds
            .map_or(point(px(0.0), px(0.0)), |bounds| bounds.origin)
    }

    fn local_screen(&self, point: Point<Pixels>) -> Point<Pixels> {
        window_to_local(point, self.canvas_origin())
    }

    pub(super) fn canvas_size(&self) -> Option<Size<Pixels>> {
        self.canvas_bounds
            .map(|bounds| bounds.size)
            .filter(|size| size.width > px(0.0) && size.height > px(0.0))
    }

    pub(super) fn window_viewport(&self) -> Viewport {
        let mut viewport = self.viewport;
        let origin = self.canvas_origin();
        let pan = viewport.pan();
        viewport.set_pan(point(pan.x + origin.x, pan.y + origin.y));
        viewport
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.smooth_zoom = None;
        self.viewport.set_zoom(zoom);
    }

    pub(super) fn queue_smooth_zoom(&mut self, factor: f32, anchor: Point<Pixels>) {
        let anchor = self.local_screen(anchor);
        let base = self
            .smooth_zoom
            .map_or(self.viewport.zoom(), |zoom| zoom.target);
        self.smooth_zoom = Some(SmoothZoom {
            target: (base * factor).clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
            anchor,
        });
    }

    pub(super) fn advance_smooth_zoom(&mut self) {
        let Some(animation) = self.smooth_zoom else {
            return;
        };
        let current = self.viewport.zoom();
        let difference = animation.target - current;
        let settled_threshold = (animation.target * 0.0005).max(0.00001);
        if difference.abs() <= settled_threshold {
            self.viewport.zoom_about(animation.anchor, animation.target);
            self.smooth_zoom = None;
        } else {
            // Cover enough distance each frame to track fresh wheel input
            // closely while retaining a short eased tail.
            self.viewport
                .zoom_about(animation.anchor, current + difference * 0.40);
        }
    }

    pub fn center_on(&mut self, id: NodeId, screen_center: Point<Pixels>) -> bool {
        let Some(position) = self.node(id).map(|node| self.node_center(node)) else {
            return false;
        };
        let screen_center = self.local_screen(screen_center);
        self.viewport.set_pan(point(
            screen_center.x - px(position.x * self.viewport.zoom()),
            screen_center.y - px(position.y * self.viewport.zoom()),
        ));
        true
    }

    pub fn fit_to_view(&mut self, screen_size: Size<Pixels>, padding: Pixels) {
        self.smooth_zoom = None;
        if !(screen_size.width / px(1.0)).is_finite()
            || !(screen_size.height / px(1.0)).is_finite()
            || !(padding / px(1.0)).is_finite()
        {
            return;
        }
        let Some(bounds) = world_bounds(&self.model.nodes, &self.model.store) else {
            return;
        };
        let available_width = ((screen_size.width - padding * 2.0) / px(1.0)).max(1.0);
        let available_height = ((screen_size.height - padding * 2.0) / px(1.0)).max(1.0);
        let zoom = (available_width / bounds.size.width.max(0.0001))
            .min(available_height / bounds.size.height.max(0.0001));
        self.viewport.set_zoom(zoom);
        let zoom = self.viewport.zoom();
        let world_center = WorldPoint::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y + bounds.size.height * 0.5,
        );
        self.viewport.set_pan(point(
            screen_size.width * 0.5 - px(world_center.x * zoom),
            screen_size.height * 0.5 - px(world_center.y * zoom),
        ));
    }
}

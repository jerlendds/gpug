use super::*;

impl Graph {
    pub fn style(&self) -> &GraphStyle {
        self.renderer.style()
    }

    pub fn set_style(&mut self, style: GraphStyle) {
        self.renderer.set_style(style);
        self.style_revision = self.style_revision.wrapping_add(1);
    }

    pub fn renderer(&self) -> &GraphRenderer {
        &self.renderer
    }

    pub fn set_layout(&mut self, layout: impl Layout + 'static) {
        self.layout = Box::new(layout);
        self.layout_initialized = false;
    }

    pub fn apply_layout_animated(&mut self, layout: impl BatchLayout + 'static, frames: usize) {
        self.set_layout(AnimatedBatchLayout::new(layout, frames));
        self.start_layout();
    }

    pub fn start_layout(&mut self) {
        self.playing = true;
    }

    pub fn stop_layout(&mut self) {
        self.playing = false;
    }

    pub fn is_layout_running(&self) -> bool {
        self.playing
    }

    pub fn layout_frame(&self) -> u64 {
        self.sim_tick
    }

    /// Converts world coordinates to GPUI window coordinates.
    ///
    /// The graph's laid-out component origin is included, so the returned
    pub(super) fn is_rendering_continuously(&self) -> bool {
        self.playing
            || self.smooth_zoom.is_some()
            || self.pan_drag_position.is_some()
            || self.drag_nodes.is_some()
    }

    /// How long this frame's simulation may run.
    ///
    /// A force-directed layout is an anytime algorithm: more steps converge
    /// faster, and stopping early costs smoothness rather than correctness.
    /// That makes it exactly the work that should absorb whatever the frame
    /// deadline has left over once drawing has been paid for, instead of
    /// taking a fixed slice that is too small on an idle frame and too large
    /// on a busy one.
    ///
    /// The configured budget stays the ceiling, so this only ever gives the
    /// simulation less than the application asked for, never more.
    fn layout_slice(&self) -> std::time::Duration {
        let configured = self.layout_options.frame_budget;
        let Some(target_ms) = self.renderer.style().frame_budget_ms else {
            return configured;
        };
        if self.last_frame_ms <= 0.0 {
            return configured;
        }
        let ceiling_ms = configured.as_secs_f32() * 1_000.0;
        let overhead_ms = (self.last_frame_ms - self.last_layout_ms).max(0.0);
        let slack_ms = (target_ms - overhead_ms).clamp(0.5, ceiling_ms);
        std::time::Duration::from_secs_f32(slack_ms / 1_000.0)
    }

    pub fn step_layout(&mut self) -> LayoutStatus {
        let _scope = profile::scope(Phase::Layout);
        let count = self.model.store.columns.len();
        self.layout_positions.clear();
        self.layout_positions.reserve(count);
        {
            let columns = &self.model.store.columns;
            self.layout_positions
                .extend((0..count).map(|index| columns.anchor(index)));
        }
        if !self.layout_initialized {
            self.layout
                .initialize(&self.layout_positions, &self.layout_edges);
            self.layout_initialized = true;
        }
        let started = std::time::Instant::now();
        let slice = self.layout_slice();
        let status = if self.layout.use_frame_budget() {
            step_with_budget(
                self.layout.as_mut(),
                &mut self.layout_positions,
                &self.layout_edges,
                slice,
            )
        } else {
            self.layout
                .step(&mut self.layout_positions, &self.layout_edges)
        };
        if matches!(&status, LayoutStatus::Failed { .. }) {
            self.playing = false;
            return status;
        }
        if matches!(&status, LayoutStatus::Converged) {
            apply_fit(&mut self.layout_positions, self.layout_options.fit);
        }
        if self.model.store.columns.is_flat() {
            // No hierarchy: an absolute position is the position. This is the
            // overwhelmingly common shape, and taking it as one branch for the
            // whole graph keeps the write-back a single sequential pass.
            for (node, absolute) in self.model.nodes.iter_mut().zip(&self.layout_positions) {
                node.position = *absolute;
            }
        } else {
            // Parents precede their children (enforced by graph validation),
            // so each parent's freshly computed origin is already in the
            // scratch column when its children read it - by index, not by a
            // hash lookup, and into storage that is reused every frame.
            self.layout_origins.clear();
            self.layout_origins.resize(count, WorldPoint::ZERO);
            let columns = &self.model.store.columns;
            for (index, (node, absolute)) in self
                .model
                .nodes
                .iter_mut()
                .zip(&self.layout_positions)
                .enumerate()
            {
                let parent = columns.parent[index];
                let parent_origin = if parent == crate::editor::NO_PARENT {
                    WorldPoint::ZERO
                } else {
                    self.layout_origins[parent as usize]
                };
                node.position =
                    WorldPoint::new(absolute.x - parent_origin.x, absolute.y - parent_origin.y);
                self.layout_origins[index] = WorldPoint::new(
                    absolute.x - columns.width[index] * node.origin.x,
                    absolute.y - columns.height[index] * node.origin.y,
                );
            }
        }
        self.model
            .store
            .sync_positions_from_specs(&self.model.nodes);
        self.last_layout_ms = started.elapsed().as_secs_f32() * 1_000.0;
        self.sim_tick = self.sim_tick.wrapping_add(1);
        if status.is_finished() {
            self.playing = false;
        }
        status
    }

    pub fn run_layout(&mut self, max_steps: usize) -> LayoutStatus {
        let original_budget = self.layout_options.frame_budget;
        self.layout_options.frame_budget = std::time::Duration::ZERO;
        let mut last_status = LayoutStatus::Running {
            energy: f32::INFINITY,
        };
        for _ in 0..max_steps {
            last_status = self.step_layout();
            if last_status.is_finished() {
                break;
            }
        }
        self.layout_options.frame_budget = original_budget;
        last_status
    }
}

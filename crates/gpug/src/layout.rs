use std::time::{Duration, Instant};

use crate::coordinates::{LayoutPoint, WorldBounds, WorldPoint};
use crate::data::LayoutEdge;
use crate::simulation::LayoutWorkspace;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutStatus {
    Running { energy: f32 },
    Converged,
}

pub trait Layout {
    fn initialize(&mut self, _positions: &[WorldPoint], _edges: &[LayoutEdge]) {}

    fn step(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> LayoutStatus;

    fn use_frame_budget(&self) -> bool {
        true
    }
}

pub trait BatchLayout {
    fn layout(&mut self, positions: &mut [LayoutPoint], edges: &[LayoutEdge])
        -> Result<(), String>;
}

pub struct BatchLayoutAdapter<L> {
    inner: L,
    finished: bool,
    error: Option<String>,
}

impl<L> BatchLayoutAdapter<L> {
    pub fn new(inner: L) -> Self {
        Self {
            inner,
            finished: false,
            error: None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl<L: BatchLayout> Layout for BatchLayoutAdapter<L> {
    fn initialize(&mut self, _positions: &[WorldPoint], _edges: &[LayoutEdge]) {
        self.finished = false;
        self.error = None;
    }

    fn step(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> LayoutStatus {
        if self.finished {
            return LayoutStatus::Converged;
        }
        let mut layout_positions: Vec<LayoutPoint> =
            positions.iter().copied().map(Into::into).collect();
        if let Err(error) = self.inner.layout(&mut layout_positions, edges) {
            self.error = Some(error);
        } else {
            for (position, layout_position) in positions.iter_mut().zip(layout_positions) {
                *position = layout_position.into();
            }
        }
        self.finished = true;
        LayoutStatus::Converged
    }
}

pub struct AnimatedBatchLayout<L> {
    batch: BatchLayoutAdapter<L>,
    start: Vec<WorldPoint>,
    target: Vec<WorldPoint>,
    current_frame: usize,
    frames: usize,
    prepared: bool,
}

impl<L> AnimatedBatchLayout<L> {
    pub fn new(layout: L, frames: usize) -> Self {
        Self {
            batch: BatchLayoutAdapter::new(layout),
            start: Vec::new(),
            target: Vec::new(),
            current_frame: 0,
            frames: frames.max(1),
            prepared: false,
        }
    }
}

impl<L: BatchLayout> Layout for AnimatedBatchLayout<L> {
    fn initialize(&mut self, positions: &[WorldPoint], _edges: &[LayoutEdge]) {
        self.start = positions.to_vec();
        self.target = positions.to_vec();
        self.current_frame = 0;
        self.prepared = false;
    }

    fn step(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> LayoutStatus {
        if !self.prepared {
            let _ = self.batch.step(&mut self.target, edges);
            self.prepared = true;
        }
        self.current_frame = (self.current_frame + 1).min(self.frames);
        let progress = self.current_frame as f32 / self.frames as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        for ((position, start), target) in positions.iter_mut().zip(&self.start).zip(&self.target) {
            position.x = start.x + (target.x - start.x) * eased;
            position.y = start.y + (target.y - start.y) * eased;
        }
        if self.current_frame == self.frames {
            LayoutStatus::Converged
        } else {
            LayoutStatus::Running {
                energy: 1.0 - progress,
            }
        }
    }

    fn use_frame_budget(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub struct ForceAtlas2 {
    workspace: LayoutWorkspace,
}

impl ForceAtlas2 {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Layout for ForceAtlas2 {
    fn step(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> LayoutStatus {
        let mut xs: Vec<_> = positions.iter().map(|position| position.x).collect();
        let mut ys: Vec<_> = positions.iter().map(|position| position.y).collect();
        let energy = self.workspace.step(&mut xs, &mut ys, edges);
        for ((position, x), y) in positions.iter_mut().zip(xs).zip(ys) {
            *position = WorldPoint::new(x, y);
        }
        if energy < 0.001 {
            LayoutStatus::Converged
        } else {
            LayoutStatus::Running { energy }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutFit {
    Preserve,
    Center,
    Fit {
        bounds: WorldBounds,
        padding: f32,
        preserve_aspect_ratio: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    pub fit: LayoutFit,
    pub frame_budget: Duration,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            fit: LayoutFit::Preserve,
            frame_budget: Duration::from_millis(4),
        }
    }
}

pub(crate) fn step_with_budget(
    layout: &mut dyn Layout,
    positions: &mut [WorldPoint],
    edges: &[LayoutEdge],
    budget: Duration,
) -> LayoutStatus {
    let started = Instant::now();
    loop {
        let status = layout.step(positions, edges);
        if status == LayoutStatus::Converged || started.elapsed() >= budget {
            return status;
        }
    }
}

pub(crate) fn apply_fit(positions: &mut [WorldPoint], fit: LayoutFit) {
    if positions.is_empty() || fit == LayoutFit::Preserve {
        return;
    }
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    for position in positions.iter() {
        min_x = min_x.min(position.x);
        max_x = max_x.max(position.x);
        min_y = min_y.min(position.y);
        max_y = max_y.max(position.y);
    }
    match fit {
        LayoutFit::Preserve => {}
        LayoutFit::Center => {
            let center_x = (min_x + max_x) * 0.5;
            let center_y = (min_y + max_y) * 0.5;
            for position in positions {
                position.x -= center_x;
                position.y -= center_y;
            }
        }
        LayoutFit::Fit {
            bounds,
            padding,
            preserve_aspect_ratio,
        } => {
            let source_width = (max_x - min_x).max(0.0001);
            let source_height = (max_y - min_y).max(0.0001);
            let target_width = (bounds.size.width - 2.0 * padding).max(0.0001);
            let target_height = (bounds.size.height - 2.0 * padding).max(0.0001);
            let mut scale_x = target_width / source_width;
            let mut scale_y = target_height / source_height;
            if preserve_aspect_ratio {
                let scale = scale_x.min(scale_y);
                scale_x = scale;
                scale_y = scale;
            }
            let fitted_width = source_width * scale_x;
            let fitted_height = source_height * scale_y;
            let offset_x = bounds.origin.x + (bounds.size.width - fitted_width) * 0.5;
            let offset_y = bounds.origin.y + (bounds.size.height - fitted_height) * 0.5;
            for position in positions {
                position.x = offset_x + (position.x - min_x) * scale_x;
                position.y = offset_y + (position.y - min_y) * scale_y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSize;

    struct OffsetBatch;

    impl BatchLayout for OffsetBatch {
        fn layout(
            &mut self,
            positions: &mut [LayoutPoint],
            _edges: &[LayoutEdge],
        ) -> Result<(), String> {
            for position in positions {
                position.x += 10.0;
                position.y += 20.0;
            }
            Ok(())
        }
    }

    #[test]
    fn fit_preserves_aspect_ratio_and_centers() {
        let mut positions = vec![WorldPoint::new(0.0, 0.0), WorldPoint::new(100.0, 50.0)];
        apply_fit(
            &mut positions,
            LayoutFit::Fit {
                bounds: WorldBounds::new(WorldPoint::ZERO, WorldSize::new(200.0, 200.0)),
                padding: 0.0,
                preserve_aspect_ratio: true,
            },
        );
        assert_eq!(
            positions,
            vec![WorldPoint::new(0.0, 50.0), WorldPoint::new(200.0, 150.0)]
        );
    }

    #[test]
    fn batch_adapter_converts_layout_coordinates_once() {
        let mut adapter = BatchLayoutAdapter::new(OffsetBatch);
        let mut positions = vec![WorldPoint::new(1.0, 2.0)];
        assert_eq!(adapter.step(&mut positions, &[]), LayoutStatus::Converged);
        assert_eq!(positions, vec![WorldPoint::new(11.0, 22.0)]);
        assert_eq!(adapter.step(&mut positions, &[]), LayoutStatus::Converged);
        assert_eq!(positions, vec![WorldPoint::new(11.0, 22.0)]);
    }

    #[test]
    fn animated_batch_layout_reaches_target_over_frames() {
        let mut layout = AnimatedBatchLayout::new(OffsetBatch, 2);
        let mut positions = vec![WorldPoint::new(0.0, 0.0)];
        layout.initialize(&positions, &[]);
        assert!(matches!(
            layout.step(&mut positions, &[]),
            LayoutStatus::Running { .. }
        ));
        assert_eq!(positions, vec![WorldPoint::new(5.0, 10.0)]);
        assert_eq!(layout.step(&mut positions, &[]), LayoutStatus::Converged);
        assert_eq!(positions, vec![WorldPoint::new(10.0, 20.0)]);
    }
}

use std::time::{Duration, Instant};

use crate::coordinates::{LayoutPoint, WorldBounds, WorldPoint};
use crate::data::LayoutEdge;
use crate::simulation::LayoutWorkspace;

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutStatus {
    Running { energy: f32 },
    Converged,
    Failed { error: String },
}

impl LayoutStatus {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Converged | Self::Failed { .. })
    }
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
            return match &self.error {
                Some(error) => LayoutStatus::Failed {
                    error: error.clone(),
                },
                None => LayoutStatus::Converged,
            };
        }
        let mut layout_positions: Vec<LayoutPoint> =
            positions.iter().copied().map(Into::into).collect();
        let status = if let Err(error) = self.inner.layout(&mut layout_positions, edges) {
            self.error = Some(error.clone());
            LayoutStatus::Failed { error }
        } else if layout_positions
            .iter()
            .any(|position| !position.x.is_finite() || !position.y.is_finite())
        {
            let error = "batch layout produced non-finite coordinates".to_string();
            self.error = Some(error.clone());
            LayoutStatus::Failed { error }
        } else {
            for (position, layout_position) in positions.iter_mut().zip(layout_positions) {
                *position = layout_position.into();
            }
            LayoutStatus::Converged
        };
        self.finished = true;
        status
    }
}

pub struct AnimatedBatchLayout<L> {
    batch: BatchLayoutAdapter<L>,
    start: Vec<WorldPoint>,
    target: Vec<WorldPoint>,
    current_frame: usize,
    frames: usize,
    prepared: bool,
    error: Option<String>,
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
            error: None,
        }
    }
}

impl<L: BatchLayout> Layout for AnimatedBatchLayout<L> {
    fn initialize(&mut self, positions: &[WorldPoint], edges: &[LayoutEdge]) {
        self.start = positions.to_vec();
        self.target = positions.to_vec();
        self.current_frame = 0;
        self.prepared = false;
        self.error = None;
        self.batch.initialize(positions, edges);
    }

    fn step(&mut self, positions: &mut [WorldPoint], edges: &[LayoutEdge]) -> LayoutStatus {
        if let Some(error) = &self.error {
            return LayoutStatus::Failed {
                error: error.clone(),
            };
        }
        if !self.prepared {
            if let LayoutStatus::Failed { error } = self.batch.step(&mut self.target, edges) {
                self.error = Some(error.clone());
                return LayoutStatus::Failed { error };
            }
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
        if positions
            .iter()
            .any(|position| !position.x.is_finite() || !position.y.is_finite())
        {
            return LayoutStatus::Failed {
                error: "layout received non-finite coordinates".into(),
            };
        }
        let energy = self.workspace.step_positions(positions, edges);
        if !energy.is_finite()
            || positions
                .iter()
                .any(|position| !position.x.is_finite() || !position.y.is_finite())
        {
            return LayoutStatus::Failed {
                error: "layout produced non-finite output".into(),
            };
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
    let mut steps = 0u32;
    loop {
        let status = layout.step(positions, edges);
        steps += 1;
        if status.is_finished() {
            return status;
        }
        // Start another step only if one is predicted to fit in what is left.
        // Checking whether the budget is already spent overshoots by a whole
        // step, and on a large graph a step is milliseconds - the difference
        // between meeting the frame deadline and missing it entirely.
        let elapsed = started.elapsed();
        if elapsed + elapsed / steps > budget {
            return status;
        }
    }
}

pub(crate) fn apply_fit(positions: &mut [WorldPoint], fit: LayoutFit) {
    if positions.is_empty() || fit == LayoutFit::Preserve {
        return;
    }
    if positions
        .iter()
        .any(|position| !position.x.is_finite() || !position.y.is_finite())
    {
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
            if !bounds.origin.x.is_finite()
                || !bounds.origin.y.is_finite()
                || !bounds.size.width.is_finite()
                || !bounds.size.height.is_finite()
                || !padding.is_finite()
            {
                return;
            }
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
mod budget_tests {
    use super::*;
    use std::time::Duration;

    struct FixedCostLayout {
        step_cost: Duration,
        steps: usize,
    }

    impl Layout for FixedCostLayout {
        fn step(&mut self, _: &mut [WorldPoint], _: &[LayoutEdge]) -> LayoutStatus {
            std::thread::sleep(self.step_cost);
            self.steps += 1;
            LayoutStatus::Running { energy: 1.0 }
        }
    }

    /// A frame budget is a deadline, not a total to be exceeded once. Checking
    /// whether the budget is already spent lets a step that has not started
    /// run anyway, overshooting by its whole duration - which on a large graph
    /// is milliseconds, and the difference between holding 60 fps and not.
    #[test]
    fn the_frame_budget_is_not_overshot_by_a_whole_step() {
        let mut layout = FixedCostLayout {
            step_cost: Duration::from_millis(4),
            steps: 0,
        };
        let mut positions = vec![WorldPoint::ZERO; 4];
        let started = std::time::Instant::now();
        step_with_budget(&mut layout, &mut positions, &[], Duration::from_millis(10));
        let elapsed = started.elapsed();

        assert_eq!(layout.steps, 2, "a third 4 ms step does not fit in 10 ms");
        assert!(
            elapsed < Duration::from_millis(10),
            "overshot the budget: {elapsed:?}"
        );
    }

    #[test]
    fn a_finished_layout_stops_regardless_of_remaining_budget() {
        struct DoneLayout(usize);
        impl Layout for DoneLayout {
            fn step(&mut self, _: &mut [WorldPoint], _: &[LayoutEdge]) -> LayoutStatus {
                self.0 += 1;
                LayoutStatus::Converged
            }
        }
        let mut layout = DoneLayout(0);
        let mut positions = vec![WorldPoint::ZERO; 2];
        step_with_budget(&mut layout, &mut positions, &[], Duration::from_secs(1));
        assert_eq!(layout.0, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldSize;

    struct OffsetBatch;

    struct FailingBatch;

    struct NonFiniteBatch;

    impl BatchLayout for FailingBatch {
        fn layout(
            &mut self,
            _positions: &mut [LayoutPoint],
            _edges: &[LayoutEdge],
        ) -> Result<(), String> {
            Err("batch layout failed".into())
        }
    }

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

    impl BatchLayout for NonFiniteBatch {
        fn layout(
            &mut self,
            positions: &mut [LayoutPoint],
            _edges: &[LayoutEdge],
        ) -> Result<(), String> {
            positions[0].x = f64::NAN;
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

    #[test]
    fn batch_adapter_reports_failure() {
        let mut adapter = BatchLayoutAdapter::new(FailingBatch);
        let mut positions = vec![WorldPoint::new(1.0, 2.0)];
        let expected = LayoutStatus::Failed {
            error: "batch layout failed".into(),
        };

        assert_eq!(adapter.step(&mut positions, &[]), expected);
        assert_eq!(adapter.step(&mut positions, &[]), expected);
        assert_eq!(positions, vec![WorldPoint::new(1.0, 2.0)]);
    }

    #[test]
    fn batch_adapter_rejects_non_finite_output() {
        let mut adapter = BatchLayoutAdapter::new(NonFiniteBatch);
        let mut positions = vec![WorldPoint::new(1.0, 2.0)];
        assert!(matches!(
            adapter.step(&mut positions, &[]),
            LayoutStatus::Failed { .. }
        ));
        assert_eq!(positions, vec![WorldPoint::new(1.0, 2.0)]);
    }

    #[test]
    fn animated_batch_layout_reports_failure() {
        let mut layout = AnimatedBatchLayout::new(FailingBatch, 2);
        let mut positions = vec![WorldPoint::new(1.0, 2.0)];
        layout.initialize(&positions, &[]);
        let expected = LayoutStatus::Failed {
            error: "batch layout failed".into(),
        };

        assert_eq!(layout.step(&mut positions, &[]), expected);
        assert_eq!(layout.step(&mut positions, &[]), expected);
        assert_eq!(positions, vec![WorldPoint::new(1.0, 2.0)]);
    }
}

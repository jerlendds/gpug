//! Frame-phase instrumentation.
//!
//! Enabled by setting `GPUG_PROFILE=1`. When disabled every scope reduces to a
//! predictable-branch load of a `bool`, so instrumentation can stay in the hot
//! render path permanently instead of being compiled in and out.
//!
//! Set `GPUG_PROFILE_INTERVAL` to change how many frames are accumulated
//! before a report is written to stderr (default 120).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// A phase of one graph frame. Ordering matches the order phases run in, so a
/// report reads top to bottom like a frame timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum Phase {
    /// Membership reconciliation at the top of `Graph::render`.
    Sync,
    /// Force-directed layout step.
    Layout,
    /// Viewport culling: which nodes survive the frustum test.
    Cull,
    /// Scene array rebuilds (positions, appearances, geometry, ordering).
    Scene,
    /// Building the retained node-content element layer.
    Content,
    /// Hit testing a pointer position against nodes, handles, and edges.
    Pick,
    /// Everything else in `Graph::render` outside the phases above.
    Render,
    /// The canvas paint closure as a whole. The three passes below are its
    /// parts, so they sum to roughly this.
    Paint,
    /// Edge geometry expansion and submission.
    PaintEdges,
    /// Node body submission.
    PaintNodes,
    /// Handles, selection, markers, resize controls, marquee.
    PaintOverlays,
    /// Barnes-Hut tree construction.
    LayoutTree,
    /// Repulsion and attraction accumulation.
    LayoutForces,
    /// Adaptive speed and position integration.
    LayoutIntegrate,
    /// Scene columns derived from node and edge specifications.
    SceneSpecs,
    /// Scene columns derived from node motion.
    SceneMotion,
    /// Scene columns derived from appearance renderers.
    SceneAppearance,
    /// Scene columns derived from selection and stacking.
    SceneSelection,
}

impl Phase {
    pub const COUNT: usize = 18;
    const NAMES: [&'static str; Self::COUNT] = [
        "sync",
        "layout",
        "cull",
        "scene",
        "content",
        "pick",
        "render",
        "paint",
        " .edges",
        " .nodes",
        " .overlay",
        " .tree",
        " .forces",
        " .integr",
        " .specs",
        " .motion",
        " .appear",
        " .select",
    ];
    #[inline]
    fn index(self) -> usize {
        self as usize
    }
}

static ENABLED: OnceLock<bool> = OnceLock::new();
static INTERVAL: OnceLock<u64> = OnceLock::new();
static NANOS: [AtomicU64; Phase::COUNT] = [const { AtomicU64::new(0) }; Phase::COUNT];
static COUNTS: [AtomicU64; Phase::COUNT] = [const { AtomicU64::new(0) }; Phase::COUNT];
static FRAMES: AtomicU64 = AtomicU64::new(0);
static COUNTERS: [AtomicU64; Counter::COUNT] = [const { AtomicU64::new(0) }; Counter::COUNT];
static FRAME_NANOS: AtomicU64 = AtomicU64::new(0);
static LAST_FRAME_START: AtomicU64 = AtomicU64::new(0);
static REPORTED: AtomicBool = AtomicBool::new(false);

/// True when `GPUG_PROFILE` requests instrumentation. Read once per process.
#[inline]
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("GPUG_PROFILE").is_ok_and(|value| value != "0" && !value.is_empty())
    })
}

fn interval() -> u64 {
    *INTERVAL.get_or_init(|| {
        std::env::var("GPUG_PROFILE_INTERVAL")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|frames| *frames > 0)
            .unwrap_or(120)
    })
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Per-frame work counters. Timings say how long a frame took; these say how
/// much the frame asked the GPU to do, which is what actually has to come down
/// when a phase is too slow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum Counter {
    /// Nodes that survived the camera cull.
    VisibleNodes,
    /// Edges submitted after culling and level-of-detail rejection.
    VisibleEdges,
    /// Edges rejected because they were off screen or sub-pixel.
    RejectedEdges,
    /// Nodes that rendered a full element tree this frame.
    ContentNodes,
    /// Calls that start a new GPU batch: one per path, one per quad run.
    DrawCalls,
    /// Triangles pushed into paths.
    Triangles,
    /// Instanced quads submitted.
    Quads,
}

impl Counter {
    pub const COUNT: usize = 7;
    const NAMES: [&'static str; Self::COUNT] = [
        "visible_nodes",
        "visible_edges",
        "rejected_edges",
        "content_nodes",
        "draw_calls",
        "triangles",
        "quads",
    ];
}

/// Adds `amount` to a per-frame counter. Inert unless profiling is enabled.
#[inline]
pub fn count(counter: Counter, amount: usize) {
    if !enabled() {
        return;
    }
    COUNTERS[counter as usize].fetch_add(amount as u64, Ordering::Relaxed);
}

/// Times one phase for as long as it is alive.
pub struct Scope {
    phase: Phase,
    start: Option<Instant>,
}

impl Scope {
    #[inline]
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            start: enabled().then(Instant::now),
        }
    }
}

impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        let elapsed = start.elapsed().as_nanos() as u64;
        NANOS[self.phase.index()].fetch_add(elapsed, Ordering::Relaxed);
        COUNTS[self.phase.index()].fetch_add(1, Ordering::Relaxed);
    }
}

/// Opens a timing scope for `phase`.
#[inline]
pub fn scope(phase: Phase) -> Scope {
    Scope::new(phase)
}

/// Records the boundary between two frames and prints a report every
/// `GPUG_PROFILE_INTERVAL` frames.
pub fn frame(nodes: usize, edges: usize, visible: usize, contents: usize) {
    if !enabled() {
        return;
    }
    let now = epoch().elapsed().as_nanos() as u64;
    let previous = LAST_FRAME_START.swap(now, Ordering::Relaxed);
    if previous != 0 {
        FRAME_NANOS.fetch_add(now.saturating_sub(previous), Ordering::Relaxed);
    }
    let frames = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if frames % interval() != 0 {
        return;
    }
    let total = FRAME_NANOS.swap(0, Ordering::Relaxed) as f64 / 1e6;
    let mut line = format!(
        "gpug frame x{frames_in_window}: {ms:.2} ms ({fps:.1} fps)  nodes {nodes} edges {edges} visible {visible} contents {contents}",
        frames_in_window = interval(),
        ms = total / interval() as f64,
        fps = 1_000.0 * interval() as f64 / total.max(0.001),
    );
    for phase in 0..Phase::COUNT {
        let nanos = NANOS[phase].swap(0, Ordering::Relaxed);
        let calls = COUNTS[phase].swap(0, Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        line.push_str(&format!(
            "\n  {:<8} {:>7.3} ms/frame  ({} calls)",
            Phase::NAMES[phase],
            nanos as f64 / 1e6 / interval() as f64,
            calls
        ));
    }
    line.push_str("\n ");
    for counter in 0..Counter::COUNT {
        let total = COUNTERS[counter].swap(0, Ordering::Relaxed);
        line.push_str(&format!(
            " {}={}",
            Counter::NAMES[counter],
            total / interval()
        ));
    }
    REPORTED.store(true, Ordering::Relaxed);
    eprintln!("{line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_inert_when_profiling_is_disabled() {
        // The test process does not set GPUG_PROFILE, so no timer is armed and
        // nothing is accumulated.
        let before = NANOS[Phase::Paint.index()].load(Ordering::Relaxed);
        {
            let _scope = scope(Phase::Paint);
        }
        assert_eq!(NANOS[Phase::Paint.index()].load(Ordering::Relaxed), before);
    }
}

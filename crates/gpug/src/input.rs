//! Ordered gesture ownership and transient pointer operations.

use crate::{NodeId, ViewportPoint, WorldPoint};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureOwner {
    Handle,
    NodeDrag,
    Marquee,
    Viewport,
    Click,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelReason {
    EntityDeleted,
    FocusLost,
    SecondTouch,
    Escape,
    RendererReplaced,
    HostRejected,
    Explicit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitTag {
    Pane,
    Node(NodeId),
    Handle,
    NoDrag,
    NoPan,
    NoWheel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Pointer { button: u8 },
    Wheel,
    Pinch,
    DoubleClick,
}

#[derive(Clone, Debug)]
pub struct GestureConfig {
    pub pan_buttons: Vec<u8>,
    pub zoom_on_wheel: bool,
    pub pan_on_scroll: bool,
    pub zoom_on_pinch: bool,
    pub zoom_on_double_click: bool,
}
impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            pan_buttons: vec![0],
            zoom_on_wheel: true,
            pan_on_scroll: false,
            zoom_on_pinch: true,
            zoom_on_double_click: true,
        }
    }
}

pub fn allows_viewport_gesture(
    kind: InputKind,
    path: &[HitTag],
    config: &GestureConfig,
    busy: bool,
) -> bool {
    if busy {
        return false;
    }
    match kind {
        InputKind::Pointer { button } => {
            config.pan_buttons.contains(&button)
                && GestureRouter::path_allows(path, GestureOwner::Viewport)
        }
        InputKind::Wheel => {
            (config.zoom_on_wheel || config.pan_on_scroll) && !path.contains(&HitTag::NoWheel)
        }
        InputKind::Pinch => config.zoom_on_pinch,
        InputKind::DoubleClick => config.zoom_on_double_click && !path.contains(&HitTag::NoPan),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Gesture {
    Idle,
    Pending {
        at: ViewportPoint,
        hit: Option<NodeId>,
    },
    NodeDrag {
        node: NodeId,
        pointer_offset: WorldPoint,
    },
    Marquee {
        start: ViewportPoint,
        current: ViewportPoint,
    },
    ViewportPan {
        previous: ViewportPoint,
    },
    Connection,
}

#[derive(Clone, Debug)]
pub struct GestureRouter {
    owner: Option<GestureOwner>,
    pub gesture: Gesture,
    pub drag_threshold: f32,
}

impl Default for GestureRouter {
    fn default() -> Self {
        Self {
            owner: None,
            gesture: Gesture::Idle,
            drag_threshold: 3.0,
        }
    }
}

impl GestureRouter {
    pub fn owner(&self) -> Option<GestureOwner> {
        self.owner
    }
    pub fn claim(&mut self, owner: GestureOwner, gesture: Gesture) -> bool {
        if self.owner.is_some() {
            return false;
        }
        self.owner = Some(owner);
        self.gesture = gesture;
        true
    }
    pub fn begin(&mut self, owner: GestureOwner, gesture: Gesture) -> bool {
        self.claim(owner, gesture)
    }
    pub fn update(&mut self, gesture: Gesture) -> bool {
        if self.owner.is_none() {
            return false;
        }
        self.gesture = gesture;
        true
    }
    pub fn end(&mut self) {
        self.finish()
    }
    pub fn cancel(&mut self, _reason: CancelReason) {
        self.finish()
    }
    pub fn finish(&mut self) {
        self.owner = None;
        self.gesture = Gesture::Idle;
    }
    pub fn path_allows(path: &[HitTag], owner: GestureOwner) -> bool {
        !path.iter().any(|tag| {
            matches!(
                (tag, owner),
                (HitTag::NoDrag, GestureOwner::NodeDrag)
                    | (HitTag::NoPan, GestureOwner::Viewport)
                    | (HitTag::NoWheel, GestureOwner::Viewport)
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerController {
    pub initial: ViewportPoint,
    pub current: ViewportPoint,
    pub threshold: f32,
    pub threshold_exceeded: bool,
    pub affected: Vec<NodeId>,
    active: bool,
}
impl PointerController {
    pub fn begin(initial: ViewportPoint, threshold: f32, affected: Vec<NodeId>) -> Self {
        Self {
            initial,
            current: initial,
            threshold,
            threshold_exceeded: threshold <= 0.0,
            affected,
            active: true,
        }
    }
    pub fn update(&mut self, current: ViewportPoint) -> bool {
        if !self.active {
            return false;
        }
        self.current = current;
        let dx = current.x - self.initial.x;
        let dy = current.y - self.initial.y;
        self.threshold_exceeded |= dx * dx + dy * dy >= self.threshold * self.threshold;
        self.threshold_exceeded
    }
    pub fn end(&mut self) -> bool {
        let completed = self.active && self.threshold_exceeded;
        self.active = false;
        completed
    }
    pub fn cancel(&mut self, _reason: CancelReason) {
        self.active = false
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_lasts_until_finish() {
        let mut r = GestureRouter::default();
        assert!(r.claim(GestureOwner::NodeDrag, Gesture::Connection));
        assert!(!r.claim(GestureOwner::Viewport, Gesture::Connection));
        r.finish();
        assert!(r.claim(GestureOwner::Viewport, Gesture::Connection));
    }
    #[test]
    fn exclusions_block_viewport_input() {
        assert!(!allows_viewport_gesture(
            InputKind::Wheel,
            &[HitTag::NoWheel],
            &GestureConfig::default(),
            false
        ));
    }
    #[test]
    fn pointer_controller_threshold_and_cancellation() {
        let mut controller =
            PointerController::begin(ViewportPoint::new(0.0, 0.0), 3.0, vec![NodeId(1)]);
        assert!(!controller.update(ViewportPoint::new(1.0, 1.0)));
        assert!(controller.update(ViewportPoint::new(4.0, 0.0)));
        controller.cancel(CancelReason::EntityDeleted);
        assert!(!controller.is_active());
    }
}

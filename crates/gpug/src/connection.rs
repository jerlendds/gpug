//! Handle resolution, edge paths, and connection gesture state.

use crate::editor::{
    ConnectionMode, Handle, HandleKey, HandleKind, HandleValidation, NodeRuntime, Position,
};
use crate::{EdgeId, NodeId, WorldPoint};
use std::sync::Arc;
pub type ConnectionValidator = Arc<dyn Fn(&Connection) -> bool + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub source: HandleKey,
    pub target: HandleKey,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionIntent {
    Create,
    ReconnectSource(EdgeId),
    ReconnectTarget(EdgeId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionState {
    Idle,
    Armed {
        from: HandleKey,
        intent: ConnectionIntent,
    },
    Connecting {
        from: HandleKey,
        to: Option<HandleKey>,
        pointer: WorldPoint,
        valid: Option<bool>,
        intent: ConnectionIntent,
    },
}

pub struct ConnectionController {
    pub state: ConnectionState,
    pub mode: ConnectionMode,
    pub radius: f32,
    validator: Option<ConnectionValidator>,
}

impl Default for ConnectionController {
    fn default() -> Self {
        Self {
            state: ConnectionState::Idle,
            mode: ConnectionMode::Strict,
            radius: 20.0,
            validator: None,
        }
    }
}

impl ConnectionController {
    pub fn set_validator(&mut self, validator: Option<ConnectionValidator>) {
        self.validator = validator;
    }
    pub fn arm(&mut self, from: HandleKey, intent: ConnectionIntent) -> bool {
        if !matches!(self.state, ConnectionState::Idle) {
            return false;
        }
        self.state = ConnectionState::Armed { from, intent };
        true
    }

    pub fn begin(&mut self, pointer: WorldPoint) -> bool {
        let ConnectionState::Armed { from, intent } = self.state.clone() else {
            return false;
        };
        self.state = ConnectionState::Connecting {
            from,
            to: None,
            pointer,
            valid: None,
            intent,
        };
        true
    }

    /// The actual hit-tested handle wins over a merely nearby candidate.
    pub fn update<'a>(
        &mut self,
        pointer: WorldPoint,
        exact: Option<&Handle>,
        nearby: impl Iterator<Item = &'a Handle>,
    ) {
        let ConnectionState::Connecting { from, intent, .. } = self.state.clone() else {
            return;
        };
        let candidate = exact
            .filter(|handle| handle.connectable_end)
            .or_else(|| closest_handle(pointer, nearby, self.radius));
        let candidate_validation = candidate.map(|handle| handle.validation);
        let to = candidate.map(|handle| handle.key.clone());
        let valid = to.as_ref().map(|to| {
            if !valid_connection(&from, to, self.mode) {
                return false;
            }
            let connection = if from.kind == HandleKind::Source {
                Connection {
                    source: from.clone(),
                    target: to.clone(),
                }
            } else {
                Connection {
                    source: to.clone(),
                    target: from.clone(),
                }
            };
            match candidate_validation.unwrap_or_default() {
                HandleValidation::Deny => false,
                HandleValidation::Allow => true,
                HandleValidation::Inherit => self
                    .validator
                    .as_ref()
                    .is_none_or(|validator| validator(&connection)),
            }
        });
        self.state = ConnectionState::Connecting {
            from,
            to,
            pointer,
            valid,
            intent,
        };
    }

    pub fn finish(&mut self) -> Option<(Connection, ConnectionIntent)> {
        let state = std::mem::replace(&mut self.state, ConnectionState::Idle);
        let ConnectionState::Connecting {
            from,
            to: Some(to),
            valid: Some(true),
            intent,
            ..
        } = state
        else {
            return None;
        };
        let connection = if from.kind == HandleKind::Source {
            Connection {
                source: from,
                target: to,
            }
        } else {
            Connection {
                source: to,
                target: from,
            }
        };
        Some((connection, intent))
    }
    pub fn end(&mut self) -> Option<(Connection, ConnectionIntent)> {
        self.finish()
    }

    pub fn cancel(&mut self) {
        self.state = ConnectionState::Idle;
    }

    /// The in-flight gesture, if any: where it started, why, and where the
    /// pointer currently is. Callers use this to report a drop that landed on
    /// the pane, which `finish` deliberately discards.
    pub fn pending(&self) -> Option<(HandleKey, ConnectionIntent, WorldPoint)> {
        let ConnectionState::Connecting {
            from,
            pointer,
            intent,
            ..
        } = &self.state
        else {
            return None;
        };
        Some((from.clone(), intent.clone(), *pointer))
    }

    pub fn click(
        &mut self,
        handle: HandleKey,
        intent: ConnectionIntent,
    ) -> Option<(Connection, ConnectionIntent)> {
        match std::mem::replace(&mut self.state, ConnectionState::Idle) {
            ConnectionState::Idle => {
                self.state = ConnectionState::Armed {
                    from: handle,
                    intent,
                };
                None
            }
            ConnectionState::Armed { from, intent } => {
                if !valid_connection(&from, &handle, self.mode) {
                    return None;
                }
                let connection = if from.kind == HandleKind::Source {
                    Connection {
                        source: from,
                        target: handle,
                    }
                } else {
                    Connection {
                        source: handle,
                        target: from,
                    }
                };
                self.validator
                    .as_ref()
                    .is_none_or(|validator| validator(&connection))
                    .then_some((connection, intent))
            }
            state => {
                self.state = state;
                None
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePosition {
    pub source: WorldPoint,
    pub target: WorldPoint,
    pub source_side: Position,
    pub target_side: Position,
}

pub fn resolve_handle<'a>(
    runtime: &'a NodeRuntime,
    id: Option<&str>,
    kind: HandleKind,
    mode: ConnectionMode,
) -> Option<&'a Handle> {
    runtime.handles.iter().find(|handle| {
        handle.key.id.as_deref() == id && (handle.key.kind == kind || mode == ConnectionMode::Loose)
    })
}

pub fn edge_position(
    source: &NodeRuntime,
    target: &NodeRuntime,
    source_id: Option<&str>,
    target_id: Option<&str>,
    mode: ConnectionMode,
) -> Option<EdgePosition> {
    let source_handle = resolve_handle(source, source_id, HandleKind::Source, mode)?;
    let target_handle = resolve_handle(target, target_id, HandleKind::Target, mode)?;
    Some(EdgePosition {
        source: source_handle.center(source.position_absolute),
        target: target_handle.center(target.position_absolute),
        source_side: source_handle.position,
        target_side: target_handle.position,
    })
}

pub fn valid_connection(from: &HandleKey, to: &HandleKey, mode: ConnectionMode) -> bool {
    from.node != to.node
        && from != to
        && (mode == ConnectionMode::Loose
            || matches!(
                (from.kind, to.kind),
                (HandleKind::Source, HandleKind::Target) | (HandleKind::Target, HandleKind::Source)
            ))
}

pub fn closest_handle<'a>(
    pointer: WorldPoint,
    handles: impl Iterator<Item = &'a Handle>,
    radius: f32,
) -> Option<&'a Handle> {
    let radius2 = radius * radius;
    handles
        .filter(|h| h.connectable_end)
        .filter_map(|handle| {
            let center = handle.center(WorldPoint::ZERO);
            let dx = center.x - pointer.x;
            let dy = center.y - pointer.y;
            let d = dx * dx + dy * dy;
            (d <= radius2).then_some((handle, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0)
}

pub fn straight_path(a: WorldPoint, b: WorldPoint) -> (Vec<WorldPoint>, WorldPoint) {
    (
        vec![a, b],
        WorldPoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5),
    )
}

pub fn bezier_path(
    a: WorldPoint,
    a_side: Position,
    b: WorldPoint,
    b_side: Position,
    curvature: f32,
) -> ([WorldPoint; 4], WorldPoint) {
    fn control(p: WorldPoint, side: Position, distance: f32) -> WorldPoint {
        match side {
            Position::Left => WorldPoint::new(p.x - distance, p.y),
            Position::Right => WorldPoint::new(p.x + distance, p.y),
            Position::Top => WorldPoint::new(p.x, p.y - distance),
            Position::Bottom => WorldPoint::new(p.x, p.y + distance),
        }
    }
    let distance = (((b.x - a.x).abs() + (b.y - a.y).abs()) * curvature).max(1.0);
    let c1 = control(a, a_side, distance);
    let c2 = control(b, b_side, distance);
    let mid = WorldPoint::new(
        (a.x + 3.0 * c1.x + 3.0 * c2.x + b.x) / 8.0,
        (a.y + 3.0 * c1.y + 3.0 * c2.y + b.y) / 8.0,
    );
    ([a, c1, c2, b], mid)
}

pub fn connection(source: NodeId, target: NodeId) -> Connection {
    Connection {
        source: HandleKey {
            node: source,
            id: None::<Arc<str>>,
            kind: HandleKind::Source,
        },
        target: HandleKey {
            node: target,
            id: None,
            kind: HandleKind::Target,
        },
    }
}

pub fn smooth_step_path(
    a: WorldPoint,
    a_side: Position,
    b: WorldPoint,
    b_side: Position,
    step: f32,
) -> (Vec<WorldPoint>, WorldPoint) {
    let distance = (b.x - a.x).abs() + (b.y - a.y).abs();
    let step = if step.is_finite() {
        step.max(0.0).min(distance * 0.25)
    } else {
        0.0
    };
    let start = match a_side {
        Position::Left => WorldPoint::new(a.x - step, a.y),
        Position::Right => WorldPoint::new(a.x + step, a.y),
        Position::Top => WorldPoint::new(a.x, a.y - step),
        Position::Bottom => WorldPoint::new(a.x, a.y + step),
    };
    let end = match b_side {
        Position::Left => WorldPoint::new(b.x - step, b.y),
        Position::Right => WorldPoint::new(b.x + step, b.y),
        Position::Top => WorldPoint::new(b.x, b.y - step),
        Position::Bottom => WorldPoint::new(b.x, b.y + step),
    };
    let center = WorldPoint::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
    let mut corners = vec![a, start];
    if matches!(a_side, Position::Left | Position::Right) {
        corners.push(WorldPoint::new(center.x, start.y));
        corners.push(WorldPoint::new(center.x, end.y))
    } else {
        corners.push(WorldPoint::new(start.x, center.y));
        corners.push(WorldPoint::new(end.x, center.y))
    }
    corners.extend([end, b]);
    corners.dedup_by(|a, b| (a.x - b.x).abs() < 0.0001 && (a.y - b.y).abs() < 0.0001);
    let mut simplified: Vec<WorldPoint> = Vec::with_capacity(corners.len());
    for point in corners {
        while simplified.len() >= 2 {
            let a = simplified[simplified.len() - 2];
            let b = simplified[simplified.len() - 1];
            let cross = (b.x - a.x) * (point.y - b.y) - (b.y - a.y) * (point.x - b.x);
            if cross.abs() >= 0.0001 {
                break;
            }
            simplified.pop();
        }
        simplified.push(point);
    }
    let corners = simplified;

    let mut points = Vec::with_capacity(corners.len() * 4);
    points.push(corners[0]);
    for window in corners.windows(3) {
        let [previous, corner, next] = [window[0], window[1], window[2]];
        let incoming = WorldPoint::new(previous.x - corner.x, previous.y - corner.y);
        let outgoing = WorldPoint::new(next.x - corner.x, next.y - corner.y);
        let incoming_length = (incoming.x * incoming.x + incoming.y * incoming.y).sqrt();
        let outgoing_length = (outgoing.x * outgoing.x + outgoing.y * outgoing.y).sqrt();
        if incoming_length <= 0.0001 || outgoing_length <= 0.0001 {
            continue;
        }
        let radius = (step * 0.5)
            .min(incoming_length * 0.5)
            .min(outgoing_length * 0.5);
        let before = WorldPoint::new(
            corner.x + incoming.x / incoming_length * radius,
            corner.y + incoming.y / incoming_length * radius,
        );
        let after = WorldPoint::new(
            corner.x + outgoing.x / outgoing_length * radius,
            corner.y + outgoing.y / outgoing_length * radius,
        );
        points.push(before);
        for sample in 1..=4 {
            let t = sample as f32 / 4.0;
            let u = 1.0 - t;
            points.push(WorldPoint::new(
                u * u * before.x + 2.0 * u * t * corner.x + t * t * after.x,
                u * u * before.y + 2.0 * u * t * corner.y + t * t * after.y,
            ));
        }
    }
    points.push(*corners.last().expect("a route always has endpoints"));
    (points, center)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_step_clamps_clearance_and_rounds_corners() {
        let a = WorldPoint::new(0.0, 0.0);
        let b = WorldPoint::new(10.0, 8.0);
        let (points, _) = smooth_step_path(a, Position::Right, b, Position::Left, 20.0);

        assert_eq!(points.first(), Some(&a));
        assert_eq!(points.last(), Some(&b));
        assert!(points.iter().all(|point| point.x >= 0.0 && point.x <= 10.0));
        assert!(points.len() > 6, "rounded corners are sampled");
    }

    #[test]
    fn smooth_step_sanitizes_non_finite_clearance() {
        let (points, _) = smooth_step_path(
            WorldPoint::new(0.0, 0.0),
            Position::Right,
            WorldPoint::new(10.0, 0.0),
            Position::Left,
            f32::NAN,
        );
        assert!(points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
    }

    #[test]
    fn strict_mode_accepts_reverse_origin_but_not_equal_kinds() {
        let c = connection(NodeId(1), NodeId(2));
        assert!(valid_connection(
            &c.source,
            &c.target,
            ConnectionMode::Strict
        ));
        assert!(valid_connection(
            &c.target,
            &c.source,
            ConnectionMode::Strict
        ));
        assert!(!valid_connection(
            &c.source,
            &HandleKey {
                node: NodeId(2),
                id: None,
                kind: HandleKind::Source
            },
            ConnectionMode::Strict
        ));
    }

    #[test]
    fn controller_finishes_only_valid_connections() {
        let c = connection(NodeId(1), NodeId(2));
        let target = Handle {
            key: c.target.clone(),
            bounds: crate::WorldBounds::new(WorldPoint::ZERO, crate::WorldSize::new(10.0, 10.0)),
            position: Position::Left,
            connectable_start: true,
            connectable_end: true,
            validation: HandleValidation::Inherit,
        };
        let mut controller = ConnectionController::default();
        assert!(controller.arm(c.source.clone(), ConnectionIntent::Create));
        assert!(controller.begin(WorldPoint::ZERO));
        controller.update(WorldPoint::ZERO, Some(&target), std::iter::empty());
        assert!(controller.finish().is_some());
        assert_eq!(controller.state, ConnectionState::Idle);
    }
    #[test]
    fn pending_reports_the_origin_of_a_drop_on_the_pane() {
        let c = connection(NodeId(1), NodeId(2));
        let mut controller = ConnectionController::default();
        controller.arm(c.source.clone(), ConnectionIntent::Create);
        controller.begin(WorldPoint::ZERO);
        controller.update(WorldPoint::new(5.0, 7.0), None, std::iter::empty());
        let (from, intent, pointer) = controller.pending().expect("gesture is in flight");
        assert_eq!(from, c.source);
        assert_eq!(intent, ConnectionIntent::Create);
        assert_eq!(pointer, WorldPoint::new(5.0, 7.0));
        assert!(controller.finish().is_none());
        assert!(controller.pending().is_none());
    }

    #[test]
    fn click_connect_uses_validator() {
        let c = connection(NodeId(1), NodeId(2));
        let mut controller = ConnectionController::default();
        controller.set_validator(Some(Arc::new(|_| false)));
        assert!(controller
            .click(c.source.clone(), ConnectionIntent::Create)
            .is_none());
        assert!(controller
            .click(c.target.clone(), ConnectionIntent::Create)
            .is_none());
        assert_eq!(controller.state, ConnectionState::Idle);
    }
    #[test]
    fn per_handle_validation_overrides_canvas_validator() {
        let c = connection(NodeId(1), NodeId(2));
        let target = Handle {
            key: c.target.clone(),
            bounds: crate::WorldBounds::new(WorldPoint::ZERO, crate::WorldSize::new(1.0, 1.0)),
            position: Position::Left,
            connectable_start: true,
            connectable_end: true,
            validation: HandleValidation::Allow,
        };
        let mut controller = ConnectionController::default();
        controller.set_validator(Some(Arc::new(|_| false)));
        controller.arm(c.source, ConnectionIntent::Create);
        controller.begin(WorldPoint::ZERO);
        controller.update(WorldPoint::ZERO, Some(&target), std::iter::empty());
        assert!(controller.finish().is_some());
    }
    #[test]
    fn exact_hit_wins_over_closer_candidate() {
        let c = connection(NodeId(1), NodeId(2));
        let exact = Handle {
            key: c.target.clone(),
            bounds: crate::WorldBounds::new(
                WorldPoint::new(10.0, 0.0),
                crate::WorldSize::new(1.0, 1.0),
            ),
            position: Position::Left,
            connectable_start: true,
            connectable_end: true,
            validation: HandleValidation::Inherit,
        };
        let nearby = Handle {
            key: HandleKey {
                node: NodeId(3),
                id: None,
                kind: HandleKind::Target,
            },
            bounds: crate::WorldBounds::new(WorldPoint::ZERO, crate::WorldSize::new(1.0, 1.0)),
            position: Position::Left,
            connectable_start: true,
            connectable_end: true,
            validation: HandleValidation::Inherit,
        };
        let mut controller = ConnectionController::default();
        controller.arm(c.source, ConnectionIntent::Create);
        controller.begin(WorldPoint::ZERO);
        controller.update(WorldPoint::ZERO, Some(&exact), std::iter::once(&nearby));
        assert!(
            matches!(&controller.state,ConnectionState::Connecting{to:Some(key),..}if key.node==NodeId(2))
        );
    }
}

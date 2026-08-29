use crate::editor::EdgeId;
use crate::node::NodeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::cell::Cell;

static NEXT_EDGE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static THREAD_ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn allocate_edge_id() -> EdgeId {
    #[cfg(test)]
    THREAD_ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
    EdgeId(
        NEXT_EDGE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("exhausted automatically allocated edge IDs"),
    )
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EdgeMarker {
    Arrow,
    ArrowClosed,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub source_handle: Option<String>,
    pub target_handle: Option<String>,
    pub edge_type: String,
    pub selected: bool,
    pub selectable: bool,
    pub deletable: bool,
    pub reconnectable: bool,
    pub interaction_width: f32,
    pub focusable: bool,
    pub label: Option<String>,
    pub marker_start: Option<EdgeMarker>,
    pub marker_end: Option<EdgeMarker>,
    pub metadata: HashMap<String, String>,
}

impl Edge {
    pub const DEFAULT_INTERACTION_WIDTH: f32 = 20.0;

    /// Creates an edge with a process-local, automatically allocated ID.
    ///
    /// The generated ID is suitable for constructing an in-memory graph, but is
    /// provisional: use [`Edge::new_with_id`] when IDs must be stable across
    /// process runs or coordinated with an external data source.
    pub fn new(source: impl Into<NodeId>, target: impl Into<NodeId>) -> Self {
        Self::new_with_id(source, target, allocate_edge_id().0)
    }

    /// Creates an edge with an explicit ID without advancing the automatic ID
    /// allocator.
    pub fn new_with_id(
        source: impl Into<NodeId>,
        target: impl Into<NodeId>,
        id: impl Into<u64>,
    ) -> Self {
        let source = source.into();
        let target = target.into();
        Self {
            id: EdgeId(id.into()),
            source,
            target,
            source_handle: None,
            target_handle: None,
            edge_type: "default".into(),
            selected: false,
            selectable: true,
            deletable: true,
            reconnectable: true,
            interaction_width: Self::DEFAULT_INTERACTION_WIDTH,
            focusable: true,
            label: None,
            marker_start: None,
            marker_end: None,
            metadata: HashMap::new(),
        }
    }
    pub fn with_id(mut self, id: impl Into<u64>) -> Self {
        self.id = EdgeId(id.into());
        self
    }
    pub fn connects(&self, other: &Self) -> bool {
        self.source == other.source
            && self.target == other.target
            && self.source_handle == other.source_handle
            && self.target_handle == other.target_handle
    }

    pub(crate) fn interaction_width_for_hit_testing(&self) -> f32 {
        crate::style::finite_non_negative_or(
            self.interaction_width,
            Self::DEFAULT_INTERACTION_WIDTH,
        )
    }
}

#[deprecated(note = "use Edge")]
pub type GpugEdge = Edge;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ids_do_not_depend_on_collision_prone_endpoint_hashes() {
        // These pairs produced the same ID under the former
        // source.rotate_left(32) ^ target scheme.
        let first = Edge::new(0u64, 0u64);
        let second = Edge::new(1u64 << 32, 1u64);

        assert_ne!(first.id, second.id);
    }

    #[test]
    fn invalid_interaction_width_uses_safe_default_for_hit_testing() {
        for width in [f32::NAN, f32::INFINITY, -1.0] {
            let mut edge = Edge::new(1u64, 2u64);
            edge.interaction_width = width;
            assert_eq!(
                edge.interaction_width_for_hit_testing(),
                Edge::DEFAULT_INTERACTION_WIDTH
            );
        }
    }

    #[test]
    fn explicit_id_constructor_does_not_advance_allocator() {
        let before = THREAD_ALLOCATIONS.with(Cell::get);
        let edge = Edge::new_with_id(1u64, 2u64, 42u64);
        let after = THREAD_ALLOCATIONS.with(Cell::get);

        assert_eq!(edge.id, EdgeId(42));
        assert_eq!(after, before);
    }
}

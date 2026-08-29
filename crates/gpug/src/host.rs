//! Host ownership, deletion policy, and diagnostic extension contracts.
use crate::{Connection, EdgeChange, EdgeId, NodeChange, NodeId};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteSet {
    pub nodes: HashSet<NodeId>,
    pub edges: HashSet<EdgeId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeleteDecision {
    Accept(DeleteSet),
    Reject,
}

pub trait GraphHost {
    fn emit_node_changes(&mut self, changes: Vec<NodeChange>);
    fn emit_edge_changes(&mut self, changes: Vec<EdgeChange>);
    fn validate_connection(&self, candidate: &Connection) -> bool;
    fn before_delete(&mut self, set: DeleteSet) -> DeleteDecision;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Diagnostic {
    UnknownNodeType(String),
    UnknownEdgeType(String),
    MissingNode(NodeId),
    MissingHandle {
        node: NodeId,
        handle: Option<String>,
    },
    InvalidConnection,
    InvalidHierarchy(String),
    MissingViewportSize,
}
pub trait DiagnosticSink: Send + Sync {
    fn report(&self, diagnostic: &Diagnostic);
}
impl<F> DiagnosticSink for F
where
    F: Fn(&Diagnostic) + Send + Sync,
{
    fn report(&self, d: &Diagnostic) {
        self(d)
    }
}
pub type SharedDiagnosticSink = Arc<dyn DiagnosticSink>;

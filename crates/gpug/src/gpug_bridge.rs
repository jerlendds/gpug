//! Conversion between analytical snapshots and GPUG editor/view data.
//!
//! Conversion allocates and validates whole snapshots: run it on a data worker,
//! then publish through `Graph::set_data` or the external-ownership API. Renderer
//! callbacks stay in the host's `GraphRenderer`; snapshots retain their type keys.
use std::fmt;

mod lod;
pub use lod::{to_lod_graph_data, LodGraphData, AGGREGATE_COUNT_KEY, INTERNAL_EDGES_KEY};

use gpug_data::{Edge, EdgeId, GraphError, GraphSnapshot, Node, NodeId, WorldRect};

/// Metadata key used for the analytical node label in GPUG nodes.
pub const LABEL_KEY: &str = "label";
/// Reserved metadata key used to retain a finite nonnegative analytical weight.
pub const WEIGHT_KEY: &str = "gpug-data.weight";

#[derive(Debug)]
pub enum BridgeError {
    Data(GraphError),
    View(crate::GraphDataError),
    MissingRuntime(NodeId),
    UnrepresentableGeometry(NodeId),
    InvalidStoredWeight(EdgeId),
}
impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(error) => error.fmt(f),
            Self::View(error) => error.fmt(f),
            Self::MissingRuntime(id) => write!(f, "missing GPUG runtime for node {}", id.0),
            Self::UnrepresentableGeometry(id) => {
                write!(f, "node {} bounds exceed finite GPUG geometry", id.0)
            }
            Self::InvalidStoredWeight(id) => {
                write!(f, "invalid analytical weight on edge {}", id.0)
            }
        }
    }
}
impl std::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Data(error) => Some(error),
            Self::View(error) => Some(error),
            _ => None,
        }
    }
}
impl From<GraphError> for BridgeError {
    fn from(error: GraphError) -> Self {
        Self::Data(error)
    }
}
impl From<crate::GraphDataError> for BridgeError {
    fn from(error: crate::GraphDataError) -> Self {
        Self::View(error)
    }
}

/// Convert to a flat GPUG view using absolute top-left positions and zero origins.
/// Stable IDs, handle pairs, renderer keys, labels, visibility and weights survive.
///
/// GPUG uses f32 world geometry: normal rounding is allowed, but overflow of any
/// coordinate, extent or endpoint is rejected. Hierarchy, selection, custom handle
/// regions and other presentation state are host-owned and are not in this data
/// schema. Configure these on the returned data before validation/publication;
/// register custom content by the preserved `node_type` in the host renderer.
pub fn to_graph_data(snapshot: &GraphSnapshot) -> Result<crate::GraphData, BridgeError> {
    let mut nodes = Vec::with_capacity(snapshot.nodes().len());
    for source in snapshot.nodes() {
        let node = to_view_node(source)?;
        nodes.push(node);
    }
    let edges = snapshot
        .edges()
        .iter()
        .map(|source| {
            let mut edge = crate::Edge::new_with_id(source.source.0, source.target.0, source.id.0);
            edge.source_handle.clone_from(&source.source_handle);
            edge.target_handle.clone_from(&source.target_handle);
            edge.metadata
                .insert(WEIGHT_KEY.into(), source.weight.to_string());
            edge
        })
        .collect();
    let data = crate::GraphData::new(nodes, edges);
    data.compile_edges()?;
    Ok(data)
}

fn to_view_node(source: &Node) -> Result<crate::Node, BridgeError> {
    let bounds = source.bounds;
    // Snapshot validation guarantees finite f64 inputs. Narrowing can still
    // overflow, including the right/bottom endpoint after f32 rounding.
    let [x, y, width, height] = [bounds.x, bounds.y, bounds.width, bounds.height].map(|v| v as f32);
    if ![x, y, width, height, x + width, y + height]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(BridgeError::UnrepresentableGeometry(source.id));
    }
    let mut node = crate::Node::new(source.id.0, crate::WorldPoint::new(x, y))
        .with_origin(crate::WorldPoint::new(0., 0.))
        .with_size(crate::WorldSize::new(width, height))
        .with_type(source.renderer.clone());
    node.hidden = source.hidden;
    node.metadata.insert(LABEL_KEY.into(), source.label.clone());
    Ok(node)
}

/// Export a coherent editor state using measured absolute runtime rectangles.
/// Parent-relative anchors and node origins are already resolved by the editor;
/// the returned analytical snapshot has no parent hierarchy. Hidden nodes remain.
///
/// Capture the editor on its owner thread before handing the completed snapshot
/// to a worker. Do not pair node specifications with runtimes from a different
/// revision. Missing runtime geometry and malformed reserved weight metadata are
/// errors. Edges without weight metadata have weight 1. Presentation-only fields
/// and arbitrary metadata are not persisted by this schema.
pub fn from_editor(editor: &crate::EditorModel) -> Result<GraphSnapshot, BridgeError> {
    capture_editor(editor)?.into_snapshot()
}

/// Owned analytical rows captured from one coherent editor state. Move this to
/// a worker to validate and build the snapshot's dense topology and CSR indexes.
/// Capture copies O(V + E) data; it is explicit and never runs during painting.
#[derive(Debug)]
pub struct SnapshotCapture {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

impl SnapshotCapture {
    /// Validate captured rows and build immutable analytical indexes on a worker.
    pub fn into_snapshot(self) -> Result<GraphSnapshot, BridgeError> {
        Ok(GraphSnapshot::new(self.nodes, self.edges)?)
    }
}

/// Copy measured absolute geometry and analytical fields on the editor's owner
/// thread. Use `SnapshotCapture::into_snapshot` on a worker for index construction.
/// Has the same schema and capture errors as `from_editor`.
pub fn capture_editor(editor: &crate::EditorModel) -> Result<SnapshotCapture, BridgeError> {
    crate::data::compile_edges(&editor.nodes, &editor.edges)?;
    let nodes = editor
        .nodes
        .iter()
        .map(|source| {
            let id = NodeId(source.id.0);
            let runtime = editor
                .store
                .runtimes
                .get(&source.id)
                .ok_or(BridgeError::MissingRuntime(id))?;
            let mut node = Node::new(
                id,
                WorldRect {
                    x: f64::from(runtime.position_absolute.x),
                    y: f64::from(runtime.position_absolute.y),
                    width: f64::from(runtime.measured.width),
                    height: f64::from(runtime.measured.height),
                },
            );
            node.label = source.metadata.get(LABEL_KEY).cloned().unwrap_or_default();
            node.renderer.clone_from(&source.node_type);
            node.hidden = source.hidden;
            Ok(node)
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    let edges = editor
        .edges
        .iter()
        .map(|source| {
            let id = EdgeId(source.id.0);
            let mut edge = Edge::new(id, source.source.0, source.target.0);
            edge.source_handle.clone_from(&source.source_handle);
            edge.target_handle.clone_from(&source.target_handle);
            edge.weight = source.metadata.get(WEIGHT_KEY).map_or(Ok(1.), |value| {
                value
                    .parse::<f64>()
                    .map_err(|_| BridgeError::InvalidStoredWeight(id))
            })?;
            Ok(edge)
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    Ok(SnapshotCapture { nodes, edges })
}

impl TryFrom<&GraphSnapshot> for crate::GraphData {
    type Error = BridgeError;

    fn try_from(snapshot: &GraphSnapshot) -> Result<Self, Self::Error> {
        to_graph_data(snapshot)
    }
}

impl crate::EditorModel {
    /// Construct an editor from a flat analytical snapshot. Presentation fields
    /// outside the analytical schema use GPUG defaults. No database is required.
    pub fn from_snapshot(
        snapshot: &GraphSnapshot,
        ownership: crate::GraphOwnership,
    ) -> Result<Self, BridgeError> {
        let data = to_graph_data(snapshot)?;
        Ok(Self::new(data.nodes, data.edges, ownership)?)
    }

    /// Capture this editor for worker-side snapshot construction. Pending external
    /// changes are not committed and therefore are not included in the capture.
    pub fn capture_snapshot(&self) -> Result<SnapshotCapture, BridgeError> {
        capture_editor(self)
    }

    /// Export measured world geometry for analysis or an explicit durable save.
    /// This builds whole-graph indexes synchronously; use `capture_snapshot` and
    /// move its result to a worker for large interactive editors.
    pub fn snapshot(&self) -> Result<GraphSnapshot, BridgeError> {
        from_editor(self)
    }

    /// Install a host-supplied analytical snapshot atomically, retaining ownership
    /// mode and middleware. This replaces presentation state with bridge defaults,
    /// just like `replace_external`, and does not emit user-edit change batches.
    pub fn replace_snapshot(&mut self, snapshot: &GraphSnapshot) -> Result<(), BridgeError> {
        let data = to_graph_data(snapshot)?;
        Ok(self.replace_external(data.nodes, data.edges)?)
    }
}

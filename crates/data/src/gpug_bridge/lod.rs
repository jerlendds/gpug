use super::{to_view_node, BridgeError, WEIGHT_KEY};
use crate::{
    spatial::{LodKind, PlanConnections, PlanEndpoint},
    Node,
};
use std::collections::HashMap;

pub const AGGREGATE_COUNT_KEY: &str = "gpug-data.aggregate_nodes";
pub const INTERNAL_EDGES_KEY: &str = "gpug-data.internal_edges";

/// Derived read-only GPUG geometry plus explicit source provenance. Node/edge
/// scene IDs must be resolved through these maps for source actions. Detailed
/// nodes retain their source IDs; aggregate IDs and group edge IDs are ephemeral.
/// Keep this object with the installed view and replace it atomically with data.
#[derive(Debug)]
pub struct LodGraphData {
    pub data: gpug::GraphData,
    connections: PlanConnections,
    node_items: HashMap<gpug::NodeId, usize>,
    edge_groups: HashMap<gpug::EdgeId, usize>,
    context_groups: Vec<usize>,
}
impl LodGraphData {
    pub fn connections(&self) -> &PlanConnections {
        &self.connections
    }
    pub fn node_item(&self, id: gpug::NodeId) -> Option<usize> {
        self.node_items.get(&id).copied()
    }
    pub fn edge_group(&self, id: gpug::EdgeId) -> Option<usize> {
        self.edge_groups.get(&id).copied()
    }
    /// Groups represented as context rather than connecting two distinct items:
    /// internal/self connections and any Outside endpoint. The host MUST surface
    /// their counts/drill-down; this list must not be treated as invisible edges.
    /// Hidden incidence is separately available from connections().hidden_edges.
    pub fn context_groups(&self) -> &[usize] {
        &self.context_groups
    }
}

/// Convert a complete node/connection plan to bounded GPUG geometry on a worker.
/// Register an explicitly distinguishable aggregate renderer under aggregate_type;
/// detailed nodes keep their original renderer keys. Aggregate node bounds are
/// the plan's clipped subtree rectangles, not estimates of individual locations.
///
/// Inter-item edges are directed count summaries, labeled with source edge counts.
/// Internal counts are placed on node metadata, and internal/outside groups are
/// retained in context_groups for a host overlay with exact source drill-down.
/// This function does NOT provide geometric paths for outside crossing edges.
/// It is a summary view, not a complete source graph or editor-session export.
///
/// Geometry uses GPUG f32 conversion checks. Data modifications are disabled by
/// default because the derived view must not be edited as if it were source data.
/// Hosts can route source edits through the provenance maps and rebuild the plan.
pub fn to_lod_graph_data(
    connections: PlanConnections,
    aggregate_type: &str,
) -> Result<LodGraphData, BridgeError> {
    let plan = connections.plan();
    let source = plan.index().snapshot();
    let mut nodes = Vec::with_capacity(plan.items().len());
    let mut node_items = HashMap::with_capacity(plan.items().len());
    let mut aggregate_ordinal = 0;
    for (item_index, item) in plan.items().iter().enumerate() {
        let mut node = match item.kind {
            LodKind::Node(id) => {
                let dense = source.node_index(id).expect("plan binds source identity");
                to_view_node(&source.nodes()[dense as usize])?
            }
            LodKind::Aggregate => {
                let mut spec = Node::new(plan.index().aggregate_id(aggregate_ordinal), item.bounds);
                aggregate_ordinal += 1;
                spec.renderer = aggregate_type.into();
                spec.label = format!("{} source nodes", item.represented_nodes);
                let mut node = to_view_node(&spec)?;
                node.metadata.insert(
                    AGGREGATE_COUNT_KEY.into(),
                    item.represented_nodes.to_string(),
                );
                node
            }
        };
        node.draggable = false;
        node.connectable = false;
        node.deletable = false;
        node_items.insert(node.id, item_index);
        nodes.push(node);
    }
    let mut edges = Vec::new();
    let mut edge_groups = HashMap::new();
    let mut context_groups = Vec::new();
    for (group_index, group) in connections.groups().iter().enumerate() {
        match (group.source, group.target) {
            (PlanEndpoint::Item(source), PlanEndpoint::Item(target)) if source != target => {
                let scene_id = u64::try_from(group_index).expect("group count fits u64");
                let mut edge =
                    gpug::Edge::new_with_id(nodes[source].id, nodes[target].id, scene_id);
                edge.label = Some(format!("{} edges", group.edge_count));
                edge.marker_end = Some(gpug::EdgeMarker::ArrowClosed);
                edge.reconnectable = false;
                edge.deletable = false;
                edge.metadata
                    .insert(WEIGHT_KEY.into(), group.weight_sum.to_string());
                edge_groups.insert(edge.id, group_index);
                edges.push(edge);
            }
            (PlanEndpoint::Item(item), PlanEndpoint::Item(_)) => {
                nodes[item]
                    .metadata
                    .insert(INTERNAL_EDGES_KEY.into(), group.edge_count.to_string());
                context_groups.push(group_index);
            }
            _ => context_groups.push(group_index),
        }
    }
    // Every source edge belongs either to one rendered group, an explicit context
    // group, or hidden incidence. Context never acquires a fabricated geometry.
    let data = gpug::GraphData::new(nodes, edges);
    data.compile_edges()?;
    Ok(LodGraphData {
        data,
        connections,
        node_items,
        edge_groups,
        context_groups,
    })
}

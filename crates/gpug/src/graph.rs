use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{canvas, div, *};

use crate::connection::{ConnectionController, ConnectionIntent, ConnectionState};
use crate::coordinates::ViewportPoint;
use crate::coordinates::{Viewport, WorldBounds, WorldPoint};
use crate::data::{GraphData, GraphDataError, LayoutEdge};
use crate::edge::Edge;
use crate::editor::{
    bounds_intersect, EditorModel, EditorStore, GraphOwnership, NodeRuntime, SelectionMode,
};
use crate::editor::{EdgeChange, NodeChange};
use crate::editor::{Handle, HandleKey, HandleKind, Position};
use crate::input::{Gesture, GestureOwner, GestureRouter, PointerController};
use crate::layout::{
    apply_fit, step_with_budget, AnimatedBatchLayout, BatchLayout, ForceAtlas2, Layout, LayoutFit,
    LayoutOptions, LayoutStatus,
};
use crate::node::{Node, NodeId};
use crate::renderer::GraphRenderer;
use crate::renderer::{EdgeAppearance, NodeAppearance, NodeShape};
use crate::style::GraphStyle;

#[derive(Clone, Copy)]
struct SmoothZoom {
    target: f32,
    anchor: Point<Pixels>,
}

struct CachedNodeContent {
    renderer: Arc<dyn crate::renderer::NodeContentRenderer>,
    node: Node,
    zoom: f32,
}

impl Render for CachedNodeContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renderer.render(&self.node, self.zoom)
    }
}

struct NodeContentItem {
    center: Point<Pixels>,
    size: Size<Pixels>,
    content: Entity<CachedNodeContent>,
}

#[derive(Default)]
struct NodeContentLayer {
    items: Vec<NodeContentItem>,
}

impl Render for NodeContentLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .size_full()
            .children(self.items.iter().map(|item| {
                div()
                    .absolute()
                    .left(item.center.x - item.size.width * 0.5)
                    .top(item.center.y - item.size.height * 0.5)
                    .w(item.size.width)
                    .h(item.size.height)
                    .child(item.content.clone())
            }))
    }
}

const CONNECTION_HANDLE_SIZE_WORLD: f32 = 0.7;
const CONNECTION_HANDLE_GAP_WORLD: f32 = 0.5;
const RECONNECT_HANDLE_SIZE_WORLD: f32 = 0.9;

/// Edge type resolved once per membership change. Comparing a copyable tag per
/// frame beats re-reading and re-hashing every edge's type string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EdgeKind {
    Straight,
    Bezier,
    SmoothStep,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AppearanceRevision {
    membership: u64,
    node_specs: u64,
    edge_specs: u64,
    selection: u64,
    style: u64,
    zoom_bits: u32,
}

impl EdgeKind {
    fn from_type(kind: &str) -> Self {
        match kind {
            "default" | "straight" => Self::Straight,
            "bezier" | "simplebezier" => Self::Bezier,
            "step" | "smoothstep" => Self::SmoothStep,
            _ => Self::Custom,
        }
    }
}

/// Per-frame paint inputs, grouped by what invalidates them. Each group carries
/// the revision stamp it was built at, so a frame that only pans the camera
/// reuses every array instead of rebuilding one entry per node and edge.
#[derive(Default)]
struct SceneCache {
    specs: Option<(u64, u64, u64)>,
    motion: Option<(u64, u64, u64)>,
    appearance: Option<AppearanceRevision>,
    selection: Option<(u64, u64)>,
    hidden: Rc<[bool]>,
    node_ids: Rc<[NodeId]>,
    node_sizes: Rc<[crate::WorldSize]>,
    edge_kinds: Rc<[EdgeKind]>,
    edge_ids: Rc<[crate::EdgeId]>,
    edge_markers: Rc<[(bool, bool)]>,
    edge_appearances: Rc<[EdgeAppearance]>,
    positions: Rc<[WorldPoint]>,
    edge_geometries: Rc<[Option<Vec<WorldPoint>>]>,
    node_appearances: Rc<[NodeAppearance]>,
    selected: Rc<[usize]>,
    node_order: Rc<[usize]>,
    selected_edges: Rc<[(crate::EdgeId, LayoutEdge)]>,
}

fn reconnecting_edge_id(state: &ConnectionState) -> Option<crate::EdgeId> {
    match state {
        ConnectionState::Connecting {
            intent: ConnectionIntent::ReconnectSource(id) | ConnectionIntent::ReconnectTarget(id),
            ..
        } => Some(*id),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub enum GraphEvent {
    NodesChanged(Vec<NodeChange>),
    EdgesChanged(Vec<EdgeChange>),
    /// Nodes removed by one delete action, together with the edges that were
    /// attached to them immediately before the action was applied.
    NodesDeleted {
        deleted: Vec<Node>,
        connected_edges: Vec<Edge>,
    },
    Connected(Edge),
    Reconnected {
        id: crate::EdgeId,
        edge: Edge,
    },
    /// A connection gesture began at `from`.
    ConnectStart {
        from: HandleKey,
        intent: ConnectionIntent,
    },
    /// A connection gesture ended. `connected` is false when the pointer was
    /// released somewhere that is not a valid handle - the pane, most often -
    /// which is the hook for "drop an edge to create a node".
    ConnectEnd {
        from: HandleKey,
        intent: ConnectionIntent,
        position: WorldPoint,
        connected: bool,
    },
    ViewportChanged(Viewport),
    SelectionChanged {
        nodes: Vec<NodeId>,
        edges: Vec<crate::EdgeId>,
    },
    Announcement(String),
}

/// The graph element under a context-menu invocation.
///
/// Hit testing follows the same precedence as primary pointer input: nodes
/// win over edges, and an otherwise empty point targets the pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextMenuTarget {
    Node(NodeId),
    Edge(crate::EdgeId),
    Selection {
        nodes: Vec<NodeId>,
        edges: Vec<crate::EdgeId>,
    },
    Pane,
}

#[derive(Default)]
struct GraphDataApiState {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    pending: HashMap<NodeId, HashMap<String, String>>,
}

/// Cloneable access to live graph connections and node metadata.
///
/// This is the Rust counterpart to React Flow's connection/data hooks and
/// `updateNodeData`: node views may query upstream nodes and queue metadata
/// patches without directly owning or mutating the graph entity.
#[derive(Clone, Default)]
pub struct GraphDataApi {
    state: Arc<Mutex<GraphDataApiState>>,
}

impl GraphDataApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node_connections(&self, node: NodeId, handle: HandleKind) -> Vec<Edge> {
        let state = self.state.lock().expect("graph data API lock poisoned");
        state
            .edges
            .iter()
            .filter(|edge| match handle {
                HandleKind::Source => edge.source == node,
                HandleKind::Target => edge.target == node,
            })
            .cloned()
            .collect()
    }

    pub fn nodes_data(&self, ids: impl IntoIterator<Item = NodeId>) -> Vec<Node> {
        let state = self.state.lock().expect("graph data API lock poisoned");
        ids.into_iter()
            .filter_map(|id| state.nodes.iter().find(|node| node.id == id).cloned())
            .collect()
    }

    pub fn node_data(&self, id: NodeId) -> Option<Node> {
        self.nodes_data([id]).into_iter().next()
    }

    pub fn update_node_data(
        &self,
        id: NodeId,
        patch: impl IntoIterator<Item = (String, String)>,
    ) -> bool {
        let mut state = self.state.lock().expect("graph data API lock poisoned");
        let patch = patch.into_iter().collect::<HashMap<_, _>>();
        let Some(node) = state.nodes.iter_mut().find(|node| node.id == id) else {
            return false;
        };
        if patch
            .iter()
            .all(|(key, value)| node.metadata.get(key) == Some(value))
        {
            return false;
        }
        node.metadata.extend(patch.clone());
        state.pending.entry(id).or_default().extend(patch);
        true
    }

    fn sync(&self, nodes: &[Node], edges: &[Edge]) {
        let mut state = self.state.lock().expect("graph data API lock poisoned");
        state.nodes.clear();
        state.nodes.extend_from_slice(nodes);
        state.edges.clear();
        state.edges.extend_from_slice(edges);
    }

    fn has_external_consumer(&self) -> bool {
        Arc::strong_count(&self.state) > 1
    }

    fn take_pending(&self) -> HashMap<NodeId, HashMap<String, String>> {
        std::mem::take(
            &mut self
                .state
                .lock()
                .expect("graph data API lock poisoned")
                .pending,
        )
    }

    fn has_pending(&self) -> bool {
        !self
            .state
            .lock()
            .expect("graph data API lock poisoned")
            .pending
            .is_empty()
    }
}

pub struct GraphBuilder {
    data: GraphData,
    layout: Box<dyn Layout>,
    layout_options: LayoutOptions,
    renderer: GraphRenderer,
    viewport: Viewport,
    interactive_layout: bool,
    fit_on_load: bool,
    show_handles: bool,
    target_handle_position: Position,
    source_handle_position: Position,
    show_resize_handles: bool,
    only_render_visible_elements: bool,
    selection_mode: SelectionMode,
    snap_grid: Option<crate::WorldSize>,
    ownership: GraphOwnership,
    auto_pan: bool,
    auto_pan_speed: f32,
    auto_pan_margin: f32,
    data_api: GraphDataApi,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self {
            data: GraphData::default(),
            layout: Box::new(ForceAtlas2::default()),
            layout_options: LayoutOptions::default(),
            renderer: GraphRenderer::default(),
            viewport: Viewport::default(),
            interactive_layout: false,
            fit_on_load: false,
            show_handles: false,
            target_handle_position: Position::Left,
            source_handle_position: Position::Right,
            show_resize_handles: false,
            only_render_visible_elements: false,
            selection_mode: SelectionMode::Partial,
            snap_grid: None,
            ownership: GraphOwnership::Internal,
            auto_pan: true,
            auto_pan_speed: Graph::DEFAULT_AUTO_PAN_SPEED,
            auto_pan_margin: Graph::DEFAULT_AUTO_PAN_MARGIN,
            data_api: GraphDataApi::default(),
        }
    }
}

impl GraphBuilder {
    pub fn data(mut self, data: GraphData) -> Self {
        self.data = data;
        self
    }

    pub fn nodes(mut self, nodes: Vec<Node>) -> Self {
        self.data.nodes = nodes;
        self
    }

    pub fn edges(mut self, edges: Vec<Edge>) -> Self {
        self.data.edges = edges;
        self
    }

    pub fn layout(mut self, layout: impl Layout + 'static) -> Self {
        self.layout = Box::new(layout);
        self
    }

    pub fn layout_options(mut self, options: LayoutOptions) -> Self {
        self.layout_options = options;
        self
    }

    pub fn layout_fit(mut self, fit: LayoutFit) -> Self {
        self.layout_options.fit = fit;
        self
    }

    pub fn style(mut self, style: GraphStyle) -> Self {
        self.renderer.set_style(style);
        self
    }

    pub fn renderer(mut self, renderer: GraphRenderer) -> Self {
        self.renderer = renderer;
        self
    }

    pub fn data_api(mut self, api: GraphDataApi) -> Self {
        self.data_api = api;
        self
    }

    pub fn viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = viewport;
        self
    }

    pub fn interactive_layout(mut self, enabled: bool) -> Self {
        self.interactive_layout = enabled;
        self
    }

    pub fn fit_on_load(mut self) -> Self {
        self.fit_on_load = true;
        self
    }

    /// Keeps connection handles visible even when their node is not selected.
    pub fn show_handles(mut self, visible: bool) -> Self {
        self.show_handles = visible;
        self
    }

    /// Places the default target and source handles on the requested sides.
    /// Existing graphs retain the conventional left-to-right arrangement.
    pub fn handle_positions(mut self, target: Position, source: Position) -> Self {
        self.target_handle_position = target;
        self.source_handle_position = source;
        self
    }

    /// Shows a south-east resize handle on selected nodes. Disabled by default.
    pub fn show_resize_handles(mut self, visible: bool) -> Self {
        self.show_resize_handles = visible;
        self
    }
    pub fn only_render_visible_elements(mut self, enabled: bool) -> Self {
        self.only_render_visible_elements = enabled;
        self
    }

    pub fn selection_mode(mut self, mode: SelectionMode) -> Self {
        self.selection_mode = mode;
        self
    }

    pub fn snap_grid(mut self, grid: crate::WorldSize) -> Self {
        self.snap_grid = Some(grid);
        self
    }

    pub fn ownership(mut self, ownership: GraphOwnership) -> Self {
        self.ownership = ownership;
        self
    }

    /// Enables camera movement when a drag, marquee, or connection gesture
    /// reaches the pane edge. Enabled by default.
    pub fn auto_pan(mut self, enabled: bool) -> Self {
        self.auto_pan = enabled;
        self
    }

    /// Screen pixels panned per frame at the very edge of the pane.
    pub fn auto_pan_speed(mut self, speed: f32) -> Self {
        self.auto_pan_speed = speed;
        self
    }

    /// Width of the pane-edge band that triggers auto-pan, in screen pixels.
    pub fn auto_pan_margin(mut self, margin: f32) -> Self {
        self.auto_pan_margin = margin;
        self
    }

    pub fn build(self, cx: &mut App) -> Result<Graph, GraphDataError> {
        Graph::from_builder(self, cx)
    }
}

/// GPUG's developer-facing graph view and orchestration entity.
pub struct Graph {
    model: EditorModel,
    layout_edges: Rc<[LayoutEdge]>,
    viewport: Viewport,
    renderer: GraphRenderer,
    layout: Box<dyn Layout>,
    layout_options: LayoutOptions,
    layout_positions: Vec<WorldPoint>,
    layout_initialized: bool,
    playing: bool,
    sim_tick: u64,
    fit_on_load_pending: bool,
    show_handles: bool,
    target_handle_position: Position,
    source_handle_position: Position,
    show_resize_handles: bool,
    pan_drag_position: Option<Point<Pixels>>,
    pointer_over_graph_item: bool,
    pointer_over_handle: bool,
    smooth_zoom: Option<SmoothZoom>,
    gestures: GestureRouter,
    selection_start: Option<Point<Pixels>>,
    selection_current: Option<Point<Pixels>>,
    drag_nodes: Option<Vec<(usize, WorldPoint)>>,
    temporary_edge_preview: Option<(WorldPoint, WorldPoint)>,
    focus: FocusHandle,
    connection: ConnectionController,
    next_edge_id: u64,
    events: Vec<GraphEvent>,
    announcement: String,
    resize_node: Option<(usize, crate::NodeResizeControl)>,
    only_render_visible_elements: bool,
    selection_mode: SelectionMode,
    snap_grid: Option<crate::WorldSize>,
    edge_geometry_cache: crate::GeometryCache<Vec<WorldPoint>>,
    scene: SceneCache,
    pointer: Option<PointerController>,
    synced_membership_revision: u64,
    style_revision: u64,
    data_api: GraphDataApi,
    data_api_sync_revision: (u64, u64, u64, u64, u64),
    node_content_cache: HashMap<NodeId, Entity<CachedNodeContent>>,
    node_content_layer: Entity<NodeContentLayer>,
    node_content_layer_revision: Option<(u64, u64, u64, u64, u64, u32, u32, u32)>,
}

impl Graph {
    /// Screen pixels panned per frame when a gesture sits at the pane edge.
    pub const DEFAULT_AUTO_PAN_SPEED: f32 = 12.0;
    /// Width of the pane-edge band that triggers auto-pan, in screen pixels.
    pub const DEFAULT_AUTO_PAN_MARGIN: f32 = 28.0;

    pub fn builder() -> GraphBuilder {
        GraphBuilder::default()
    }

    pub fn from_data(data: GraphData, cx: &mut App) -> Result<Self, GraphDataError> {
        Self::builder().data(data).build(cx)
    }

    /// Compatibility constructor for the original prototype. `k` and `beta`
    /// are ignored; generator configuration belongs outside the graph view.
    #[deprecated(note = "use Graph::builder() or Graph::from_data()")]
    pub fn new(cx: &mut App, nodes: Vec<Node>, edges: Vec<Edge>, _k: usize, _beta: f32) -> Self {
        Self::builder()
            .nodes(nodes)
            .edges(edges)
            .build(cx)
            .expect("Graph::new received invalid graph data")
    }

    fn from_builder(builder: GraphBuilder, cx: &mut App) -> Result<Self, GraphDataError> {
        let layout_edges: Rc<[LayoutEdge]> = builder.data.compile_edges()?.into();
        let next_edge_id = builder
            .data
            .edges
            .iter()
            .map(|edge| edge.id.0)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        let model = EditorModel::new(builder.data.nodes, builder.data.edges, builder.ownership)?;
        builder.data_api.sync(&model.nodes, &model.edges);
        let synced_membership_revision = model.store.dirty.revisions.membership;
        let revisions = model.store.dirty.revisions;
        let node_content_layer = cx.new(|_| NodeContentLayer::default());
        Ok(Self {
            model,
            layout_edges,
            viewport: builder.viewport,
            renderer: builder.renderer,
            layout: builder.layout,
            layout_options: builder.layout_options,
            layout_positions: Vec::new(),
            layout_initialized: false,
            playing: builder.interactive_layout,
            sim_tick: 0,
            fit_on_load_pending: builder.fit_on_load,
            show_handles: builder.show_handles,
            target_handle_position: builder.target_handle_position,
            source_handle_position: builder.source_handle_position,
            show_resize_handles: builder.show_resize_handles,
            pan_drag_position: None,
            pointer_over_graph_item: false,
            pointer_over_handle: false,
            smooth_zoom: None,
            gestures: GestureRouter::default(),
            selection_start: None,
            selection_current: None,
            drag_nodes: None,
            temporary_edge_preview: None,
            focus: cx.focus_handle().tab_stop(true),
            connection: ConnectionController::default(),
            next_edge_id,
            events: Vec::new(),
            announcement: String::new(),
            resize_node: None,
            only_render_visible_elements: builder.only_render_visible_elements,
            selection_mode: builder.selection_mode,
            snap_grid: builder.snap_grid,
            edge_geometry_cache: crate::GeometryCache::default(),
            scene: SceneCache::default(),
            pointer: None,
            synced_membership_revision,
            style_revision: 0,
            data_api: builder.data_api,
            data_api_sync_revision: (
                revisions.membership,
                revisions.node_specs,
                revisions.edge_specs,
                revisions.nodes,
                revisions.selection,
            ),
            node_content_cache: HashMap::new(),
            node_content_layer,
            node_content_layer_revision: None,
        })
    }

    pub fn set_data(&mut self, data: GraphData) -> Result<(), GraphDataError> {
        data.compile_edges()?;
        self.model.replace_external(data.nodes, data.edges)?;
        self.layout_initialized = false;
        self.sim_tick = 0;
        Ok(())
    }

    pub fn replace_external(
        &mut self,
        nodes: Vec<Node>,
        edges: Vec<Edge>,
    ) -> Result<(), GraphDataError> {
        let data = GraphData::new(nodes, edges);
        data.compile_edges()?;
        self.model.replace_external(data.nodes, data.edges)?;
        self.layout_initialized = false;
        self.sim_tick = 0;
        Ok(())
    }

    pub fn add_node(&mut self, node: Node) -> Result<(), GraphDataError> {
        self.model.nodes.push(node);
        let validation = crate::data::compile_edges(&self.model.nodes, &self.model.edges);
        let node = self.model.nodes.pop().expect("node was just pushed");
        validation?;
        self.model.emit_nodes([NodeChange::Add {
            index: None,
            item: node,
        }])?;
        Ok(())
    }

    pub fn add_edge(&mut self, edge: Edge) -> Result<(), GraphDataError> {
        self.model.edges.push(edge);
        let validation = crate::data::compile_edges(&self.model.nodes, &self.model.edges);
        let edge = self.model.edges.pop().expect("edge was just pushed");
        validation?;
        self.model.emit_edges([EdgeChange::Add {
            index: None,
            item: edge,
        }])?;
        Ok(())
    }

    pub fn nodes(&self) -> &[Node] {
        &self.model.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.model.edges
    }

    /// Sets transient edge geometry painted by the graph without adding it to
    /// persistent graph data. Passing `None` clears the preview.
    pub fn set_temporary_edge_preview(&mut self, preview: Option<(WorldPoint, WorldPoint)>) {
        self.temporary_edge_preview = preview;
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.model.node(id)
    }

    fn node_center(&self, node: &Node) -> WorldPoint {
        self.model.store.node_center_absolute(node)
    }

    pub fn set_node_position(
        &mut self,
        id: NodeId,
        position: WorldPoint,
    ) -> Result<bool, GraphDataError> {
        let Some(index) = self.model.store.node_lookup.get(&id).copied() else {
            return Ok(false);
        };
        let relative = self
            .model
            .store
            .position_relative_to_parent(&self.model.nodes[index], position);
        self.model.emit_nodes([NodeChange::Position {
            id,
            position: Some(relative),
            dragging: Some(false),
        }])?;
        Ok(true)
    }

    pub fn editor(&self) -> &EditorStore {
        &self.model.store
    }

    /// Returns the measured rectangle occupied by a node in world space.
    pub fn node_bounds(&self, id: NodeId) -> Option<WorldBounds> {
        self.model.store.runtimes.get(&id).map(NodeRuntime::bounds)
    }

    /// Starts a custom resize gesture at a world-space pointer position.
    pub fn begin_node_resize(
        &mut self,
        control: &mut crate::NodeResizeControl,
        pointer: WorldPoint,
    ) -> bool {
        let Some(bounds) = self.node_bounds(control.node_id()) else {
            return false;
        };
        control.begin(pointer, bounds);
        self.announce("Resizing node");
        true
    }

    /// Applies the current pointer position for a custom resize gesture.
    pub fn update_node_resize(
        &mut self,
        control: &crate::NodeResizeControl,
        pointer: WorldPoint,
    ) -> bool {
        let Some(bounds) = control.update(pointer) else {
            return false;
        };
        self.model
            .resize_node_from_bounds(control.node_id(), bounds, true)
    }

    /// Finishes a custom resize gesture and emits its final dimensions.
    pub fn end_node_resize(
        &mut self,
        control: &mut crate::NodeResizeControl,
        pointer: WorldPoint,
    ) -> bool {
        let Some(bounds) = control.end(pointer) else {
            return false;
        };
        let changed = self
            .model
            .resize_node_from_bounds(control.node_id(), bounds, false);
        if changed {
            self.announce("Node resize finished");
        }
        changed
    }

    /// Tests a node's measured rectangle against a world-space area.
    ///
    /// Hidden or not-yet-measured nodes do not intersect. `Partial` accepts
    /// any overlap (including touching edges); `Full` requires the area to
    /// completely contain the node.
    pub fn is_node_intersecting(&self, id: NodeId, area: WorldBounds, mode: SelectionMode) -> bool {
        self.model
            .node(id)
            .filter(|node| !node.hidden)
            .and_then(|_| self.node_bounds(id))
            .is_some_and(|bounds| bounds_intersect(area, bounds, mode))
    }

    /// Finds visible, measured nodes intersecting a world-space area.
    pub fn intersecting_nodes(&self, area: WorldBounds, mode: SelectionMode) -> HashSet<NodeId> {
        self.model
            .nodes
            .iter()
            .filter(|node| !node.hidden && self.is_node_intersecting(node.id, area, mode))
            .map(|node| node.id)
            .collect()
    }

    /// Finds nodes intersecting `id`, excluding that node from the result.
    pub fn get_intersecting_nodes(&self, id: NodeId, mode: SelectionMode) -> HashSet<NodeId> {
        let Some(bounds) = self.node_bounds(id) else {
            return HashSet::new();
        };
        let mut result = self.intersecting_nodes(bounds, mode);
        result.remove(&id);
        result
    }

    fn sync(&mut self) {
        let membership_revision = self.model.store.dirty.revisions.membership;
        if !self.model.store.dirty.peek().membership
            || membership_revision == self.synced_membership_revision
        {
            return;
        }
        self.layout_edges = crate::data::compile_edges(&self.model.nodes, &self.model.edges)
            .expect("EditorModel invariants were violated after validated mutation")
            .into();
        self.edge_geometry_cache
            .retain(&self.model.edges.iter().map(|edge| edge.id).collect());
        self.synced_membership_revision = membership_revision;
    }

    pub fn measure_node(
        &mut self,
        id: NodeId,
        size: crate::WorldSize,
        handles: Vec<crate::Handle>,
    ) -> bool {
        self.model.store.measure_node(id, size, handles)
    }

    pub fn selected_nodes(&self) -> impl Iterator<Item = &Node> {
        self.model
            .nodes
            .iter()
            .filter(|node| self.model.store.node_selected(node))
    }

    /// Hit-tests a screen-space point for application-owned context menus.
    pub fn context_menu_target(&self, position: Point<Pixels>) -> ContextMenuTarget {
        if let Some(index) = self.node_at_screen_position(position) {
            let node = &self.model.nodes[index];
            let selected_nodes = self
                .model
                .nodes
                .iter()
                .filter(|node| self.model.store.node_selected(node))
                .map(|node| node.id)
                .collect::<Vec<_>>();
            let selected_edges = self
                .model
                .edges
                .iter()
                .filter(|edge| self.model.store.edge_selected(edge))
                .map(|edge| edge.id)
                .collect::<Vec<_>>();
            if self.model.store.node_selected(node)
                && selected_nodes.len() + selected_edges.len() > 1
            {
                ContextMenuTarget::Selection {
                    nodes: selected_nodes,
                    edges: selected_edges,
                }
            } else {
                ContextMenuTarget::Node(node.id)
            }
        } else if let Some(index) = self.edge_index_at_screen_position(position) {
            let edge = &self.model.edges[index];
            let selected_nodes = self
                .model
                .nodes
                .iter()
                .filter(|node| self.model.store.node_selected(node))
                .map(|node| node.id)
                .collect::<Vec<_>>();
            let selected_edges = self
                .model
                .edges
                .iter()
                .filter(|edge| self.model.store.edge_selected(edge))
                .map(|edge| edge.id)
                .collect::<Vec<_>>();
            if self.model.store.edge_selected(edge)
                && selected_nodes.len() + selected_edges.len() > 1
            {
                ContextMenuTarget::Selection {
                    nodes: selected_nodes,
                    edges: selected_edges,
                }
            } else {
                ContextMenuTarget::Edge(edge.id)
            }
        } else {
            ContextMenuTarget::Pane
        }
    }

    /// Selects one node, replacing the current selection.
    pub fn select_node(&mut self, id: NodeId) -> bool {
        self.model.select_node(id, false, false)
    }

    /// Selects one edge, replacing the current selection.
    pub fn select_edge(&mut self, id: crate::EdgeId) -> bool {
        self.model.select_edge(id, false, false)
    }

    /// Deletes the selected, deletable graph elements.
    pub fn delete_selected(&mut self) -> bool {
        self.delete_selected_and_emit()
    }

    fn delete_selected_and_emit(&mut self) -> bool {
        let deleted = self
            .model
            .nodes
            .iter()
            .filter(|node| self.model.store.node_selected(node) && node.deletable)
            .cloned()
            .collect::<Vec<_>>();
        let deleted_ids = deleted.iter().map(|node| node.id).collect::<HashSet<_>>();
        let connected_edges = self
            .model
            .edges
            .iter()
            .filter(|edge| deleted_ids.contains(&edge.source) || deleted_ids.contains(&edge.target))
            .cloned()
            .collect::<Vec<_>>();
        let changed = self.model.delete_selected(|_, _| true);
        if changed && !deleted.is_empty() {
            self.events.push(GraphEvent::NodesDeleted {
                deleted,
                connected_edges,
            });
        }
        changed
    }

    pub fn take_events(&mut self) -> Vec<GraphEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn take_dirty(&mut self) -> crate::DirtySet {
        self.model.store.take_dirty()
    }
    pub fn revisions(&self) -> crate::Revisions {
        self.model.store.dirty.revisions
    }
    pub fn announcement(&self) -> &str {
        &self.announcement
    }

    fn announce(&mut self, message: impl Into<String>) {
        self.announcement = message.into();
        self.events
            .push(GraphEvent::Announcement(self.announcement.clone()));
    }

    fn emit_selection(&mut self) {
        self.model.store.dirty.mark_selection();
        self.events.push(GraphEvent::SelectionChanged {
            nodes: self
                .model
                .nodes
                .iter()
                .filter(|node| self.model.store.node_selected(node))
                .map(|node| node.id)
                .collect(),
            edges: self
                .model
                .edges
                .iter()
                .filter(|edge| self.model.store.edge_selected(edge))
                .map(|edge| edge.id)
                .collect(),
        });
    }

    fn flush(&mut self) {
        let (nodes, edges) = self.model.take_changes();
        if !nodes.is_empty() {
            self.events.push(GraphEvent::NodesChanged(nodes));
        }
        if !edges.is_empty() {
            self.events.push(GraphEvent::EdgesChanged(edges));
        }
        if self.model.store.dirty.peek().overlays {
            self.emit_selection();
        }
    }

    pub fn clear_selection(&mut self) {
        self.model.clear_selection();
    }

    fn handle_key(&mut self, event: &KeyDownEvent) -> bool {
        let step = if event.keystroke.modifiers.shift {
            10.0
        } else {
            1.0
        };
        match event.keystroke.key.as_str() {
            "left" | "right" | "up" | "down" => {
                let delta = match event.keystroke.key.as_str() {
                    "left" => WorldPoint::new(-step, 0.0),
                    "right" => WorldPoint::new(step, 0.0),
                    "up" => WorldPoint::new(0.0, -step),
                    _ => WorldPoint::new(0.0, step),
                };
                self.model.move_selected(delta, self.snap_grid, false);
                self.announce(format!("Moved selected nodes by {}, {}", delta.x, delta.y));
                true
            }
            "escape" => {
                self.model.clear_selection();
                self.announce("Selection cleared");
                true
            }
            "backspace" | "delete" => {
                self.delete_selected_and_emit();
                self.announce("Deleted selected graph elements");
                true
            }
            "a" if event.keystroke.modifiers.control || event.keystroke.modifiers.platform => {
                self.model.select_all();
                self.announce("Selected all graph elements");
                true
            }
            _ => false,
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.smooth_zoom = None;
        self.events.push(GraphEvent::ViewportChanged(viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn sync_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.smooth_zoom = None;
        self.model.store.dirty.mark_viewport();
    }

    pub fn set_pan(&mut self, pan: Point<Pixels>) {
        self.viewport.set_pan(pan);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }

    pub fn pan_by(&mut self, delta: Point<Pixels>) {
        let pan = self.viewport.pan();
        self.viewport
            .set_pan(point(pan.x + delta.x, pan.y + delta.y));
        self.model.store.dirty.mark_viewport();
    }

    pub fn zoom_in(&mut self, anchor: Point<Pixels>) {
        self.viewport.zoom_about(anchor, self.viewport.zoom() * 1.2);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn zoom_out(&mut self, anchor: Point<Pixels>) {
        self.viewport.zoom_about(anchor, self.viewport.zoom() / 1.2);
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn set_center(&mut self, world: WorldPoint, screen_size: Size<Pixels>, zoom: f32) {
        self.viewport.set_center(
            world,
            crate::WorldSize::new(screen_size.width / px(1.0), screen_size.height / px(1.0)),
            zoom,
        );
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn fit_bounds(&mut self, bounds: WorldBounds, screen_size: Size<Pixels>, padding: Pixels) {
        self.viewport.fit_bounds(
            bounds,
            crate::WorldSize::new(screen_size.width / px(1.0), screen_size.height / px(1.0)),
            padding / px(1.0),
        );
        self.events.push(GraphEvent::ViewportChanged(self.viewport));
        self.model.store.dirty.mark_viewport();
    }
    pub fn screen_to_flow_position(
        &self,
        point: Point<Pixels>,
        snap: Option<crate::WorldSize>,
    ) -> WorldPoint {
        self.viewport.viewport_to_world(
            ViewportPoint::new(point.x / px(1.0), point.y / px(1.0)),
            snap,
        )
    }
    pub fn flow_to_screen_position(&self, point: WorldPoint) -> Point<Pixels> {
        self.viewport.world_to_screen(point)
    }

    fn node_at_screen_position(&self, position: Point<Pixels>) -> Option<usize> {
        let hit_radius = px(self.renderer.style().hit_radius_pixels);
        self.model
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                if node.hidden {
                    return None;
                }
                let center = self.viewport.world_to_screen(self.node_center(node));
                let (half_width, half_height) = if self.renderer.has_node_content(node) {
                    let measured = self
                        .model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(node.size, |runtime| runtime.measured);
                    (
                        px(measured.width * self.viewport.zoom() * 0.5),
                        px(measured.height * self.viewport.zoom() * 0.5),
                    )
                } else {
                    (hit_radius, hit_radius)
                };
                let hit = (center.x - position.x).abs() <= half_width
                    && (center.y - position.y).abs() <= half_height;
                hit.then_some((
                    self.model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(0, |runtime| runtime.z),
                    index,
                ))
            })
            .max()
            .map(|(_, index)| index)
    }

    fn node_allows_drag_at_screen_position(&self, node: &Node, position: Point<Pixels>) -> bool {
        if !node.draggable {
            return false;
        }
        let pointer = self.screen_to_world(position);
        let absolute = self.model.store.node_position_absolute(node);
        let top_left = WorldPoint::new(
            absolute.x - node.size.width * node.origin.x,
            absolute.y - node.size.height * node.origin.y,
        );
        let local = WorldPoint::new(pointer.x - top_left.x, pointer.y - top_left.y);
        node.allows_drag_at(local)
    }

    fn handle_at_screen_position(
        &self,
        position: Point<Pixels>,
        end: bool,
    ) -> Option<(HandleKey, WorldPoint)> {
        let hit = px(CONNECTION_HANDLE_SIZE_WORLD * self.viewport.zoom());
        self.model
            .nodes
            .iter()
            .filter_map(|node| {
                if !node.connectable || node.hidden {
                    return None;
                }
                let center = self.viewport.world_to_screen(self.node_center(node));
                let kind = if end {
                    HandleKind::Target
                } else {
                    HandleKind::Source
                };
                if node.connectable_body {
                    let measured = self
                        .model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(node.size, |runtime| runtime.measured);
                    let half_width = px(measured.width * self.viewport.zoom() * 0.5);
                    let half_height = px(measured.height * self.viewport.zoom() * 0.5);
                    let inside = (center.x - position.x).abs() <= half_width
                        && (center.y - position.y).abs() <= half_height;
                    // A whole-body port otherwise consumes every pointer down.
                    // Reserve an explicit custom drag handle for node movement,
                    // while still accepting drops over that region.
                    let reserved_for_drag = !end
                        && node.custom_handle.is_some()
                        && self.node_allows_drag_at_screen_position(node, position);
                    if inside && !reserved_for_drag {
                        return Some((
                            0.0,
                            HandleKey {
                                node: node.id,
                                id: None,
                                kind,
                            },
                            self.node_center(node),
                        ));
                    }
                }
                if !self.show_handles && !self.model.store.node_selected(node) {
                    return None;
                }
                let handle_position = connection_handle_position(
                    center,
                    self.renderer
                        .node_appearance(node, self.viewport.zoom())
                        .radius_pixels,
                    kind,
                    self.target_handle_position,
                    self.source_handle_position,
                    self.viewport.zoom(),
                );
                let dx = (handle_position.x - position.x).abs();
                let dy = (handle_position.y - position.y).abs();
                let center_distance = (center.x - position.x).abs();
                // The fallback handles can be close to the node center at low
                // zoom. Do not let their generous hit box swallow the endpoint
                // hotspot used for reconnecting a selected edge.
                (dx <= hit && dy <= hit && dx < center_distance).then_some((
                    (dx / px(1.0)).powi(2) + (dy / px(1.0)).powi(2),
                    HandleKey {
                        node: node.id,
                        id: None,
                        kind,
                    },
                    self.viewport.screen_to_world(handle_position),
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, key, center)| (key, center))
    }

    fn is_handle_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.handle_at_screen_position(position, false).is_some()
            || self.handle_at_screen_position(position, true).is_some()
    }

    fn reconnect_at_screen_position(
        &self,
        position: Point<Pixels>,
    ) -> Option<(HandleKey, ConnectionIntent)> {
        let hit = px(RECONNECT_HANDLE_SIZE_WORLD * self.viewport.zoom());
        self.model
            .edges
            .iter()
            .filter(|edge| self.model.store.edge_selected(edge) && edge.reconnectable)
            .find_map(|edge| {
                let source = self.model.node(edge.source)?;
                let target = self.model.node(edge.target)?;
                let source_point = self.viewport.world_to_screen(self.node_center(source));
                let target_point = self.viewport.world_to_screen(self.node_center(target));
                if (source_point.x - position.x).abs() <= hit
                    && (source_point.y - position.y).abs() <= hit
                {
                    Some((
                        HandleKey {
                            node: target.id,
                            id: edge.target_handle.clone().map(Into::into),
                            kind: HandleKind::Target,
                        },
                        ConnectionIntent::ReconnectSource(edge.id),
                    ))
                } else if (target_point.x - position.x).abs() <= hit
                    && (target_point.y - position.y).abs() <= hit
                {
                    Some((
                        HandleKey {
                            node: source.id,
                            id: edge.source_handle.clone().map(Into::into),
                            kind: HandleKind::Source,
                        },
                        ConnectionIntent::ReconnectTarget(edge.id),
                    ))
                } else {
                    None
                }
            })
    }

    fn resize_at_screen_position(
        &self,
        position: Point<Pixels>,
    ) -> Option<(usize, crate::ResizeDirection)> {
        if !self.show_resize_handles {
            return None;
        }
        self.model
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                self.model.store.node_selected(node) || node.resize_controls_always_visible
            })
            .find_map(|(index, node)| {
                let hit = px(node.resize_control_hit_radius);
                let center = self.viewport.world_to_screen(self.node_center(node));
                node.resize_directions
                    .as_deref()
                    .unwrap_or(&RESIZE_DIRECTIONS)
                    .iter()
                    .copied()
                    .find_map(|direction| {
                        let handle = resize_handle_position(
                            center,
                            node.size,
                            self.viewport.zoom(),
                            direction,
                        );
                        ((handle.x - position.x).abs() <= hit
                            && (handle.y - position.y).abs() <= hit)
                            .then_some((index, direction))
                    })
            })
    }

    fn connection_handle(&self, key: HandleKey, center: WorldPoint) -> Handle {
        let position = if key.kind == HandleKind::Target {
            self.target_handle_position
        } else {
            self.source_handle_position
        };
        Handle {
            key,
            bounds: WorldBounds::new(center, crate::WorldSize::new(0.0, 0.0)),
            position,
            connectable_start: true,
            connectable_end: true,
            validation: crate::editor::HandleValidation::Inherit,
        }
    }

    fn edge_index_at_screen_position(&self, position: Point<Pixels>) -> Option<usize> {
        let point_x = position.x / px(1.0);
        let point_y = position.y / px(1.0);
        // Hit testing can run between a structural edit and the next render,
        // before `sync` has rebuilt `layout_edges`. Use the live edge list and
        // stable endpoint IDs so deleting an edge cannot leave parallel arrays
        // with mismatched lengths or ordering here.
        self.model
            .edges
            .iter()
            .enumerate()
            .find_map(|(edge_index, edge)| {
                let source = self.model.node(edge.source)?;
                let target = self.model.node(edge.target)?;
                let tolerance = edge.interaction_width_for_hit_testing() * 0.5;
                let tolerance_squared = tolerance.powi(2);
                let start = self.viewport.world_to_screen(self.node_center(source));
                let end = self.viewport.world_to_screen(self.node_center(target));
                let start_x = start.x / px(1.0);
                let start_y = start.y / px(1.0);
                let dx = end.x / px(1.0) - start_x;
                let dy = end.y / px(1.0) - start_y;
                let length_squared = dx * dx + dy * dy;
                let t = if length_squared > 0.0 {
                    (((point_x - start_x) * dx + (point_y - start_y) * dy) / length_squared)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest_x = start_x + t * dx;
                let nearest_y = start_y + t * dy;
                ((point_x - nearest_x).powi(2) + (point_y - nearest_y).powi(2) <= tolerance_squared)
                    .then_some(edge_index)
            })
    }

    fn edge_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.edge_index_at_screen_position(position).is_some()
    }

    fn graph_item_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.node_at_screen_position(position).is_some() || self.edge_at_screen_position(position)
    }

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

    pub fn world_to_screen(&self, point: WorldPoint) -> Point<Pixels> {
        self.viewport.world_to_screen(point)
    }

    pub fn screen_to_world(&self, point: Point<Pixels>) -> WorldPoint {
        self.viewport.screen_to_world(point)
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.smooth_zoom = None;
        self.viewport.set_zoom(zoom);
    }

    fn queue_smooth_zoom(&mut self, factor: f32, anchor: Point<Pixels>) {
        let base = self
            .smooth_zoom
            .map_or(self.viewport.zoom(), |zoom| zoom.target);
        self.smooth_zoom = Some(SmoothZoom {
            target: (base * factor).clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
            anchor,
        });
    }

    fn advance_smooth_zoom(&mut self) {
        let Some(animation) = self.smooth_zoom else {
            return;
        };
        let current = self.viewport.zoom();
        let difference = animation.target - current;
        let settled_threshold = (animation.target * 0.0005).max(0.00001);
        if difference.abs() <= settled_threshold {
            self.viewport.zoom_about(animation.anchor, animation.target);
            self.smooth_zoom = None;
        } else {
            // Cover enough distance each frame to track fresh wheel input
            // closely while retaining a short eased tail.
            self.viewport
                .zoom_about(animation.anchor, current + difference * 0.40);
        }
    }

    pub fn center_on(&mut self, id: NodeId, screen_center: Point<Pixels>) -> bool {
        let Some(position) = self.node(id).map(|node| self.node_center(node)) else {
            return false;
        };
        self.viewport.set_pan(point(
            screen_center.x - px(position.x * self.viewport.zoom()),
            screen_center.y - px(position.y * self.viewport.zoom()),
        ));
        true
    }

    pub fn fit_to_view(&mut self, screen_size: Size<Pixels>, padding: Pixels) {
        self.smooth_zoom = None;
        if !(screen_size.width / px(1.0)).is_finite()
            || !(screen_size.height / px(1.0)).is_finite()
            || !(padding / px(1.0)).is_finite()
        {
            return;
        }
        let Some(bounds) = world_bounds(&self.model.nodes, &self.model.store) else {
            return;
        };
        let available_width = ((screen_size.width - padding * 2.0) / px(1.0)).max(1.0);
        let available_height = ((screen_size.height - padding * 2.0) / px(1.0)).max(1.0);
        let zoom = (available_width / bounds.size.width.max(0.0001))
            .min(available_height / bounds.size.height.max(0.0001));
        self.viewport.set_zoom(zoom);
        let zoom = self.viewport.zoom();
        let world_center = WorldPoint::new(
            bounds.origin.x + bounds.size.width * 0.5,
            bounds.origin.y + bounds.size.height * 0.5,
        );
        self.viewport.set_pan(point(
            screen_size.width * 0.5 - px(world_center.x * zoom),
            screen_size.height * 0.5 - px(world_center.y * zoom),
        ));
    }

    /// Rebuilds only the paint arrays whose inputs changed since the last
    /// frame. Panning, hovering, and idling reuse all of them; a layout frame
    /// rebuilds positions and edge geometry alone.
    fn refresh_scene(&mut self, renderer: &GraphRenderer, zoom: f32) {
        let revisions = self.model.store.dirty.revisions;
        let membership = revisions.membership;
        let specs = (membership, revisions.node_specs, revisions.edge_specs);
        if self.scene.specs != Some(specs) {
            self.scene.specs = Some(specs);
            self.scene.hidden = self.model.nodes.iter().map(|node| node.hidden).collect();
            self.scene.node_ids = self.model.nodes.iter().map(|node| node.id).collect();
            self.scene.node_sizes = self
                .model
                .nodes
                .iter()
                .map(|node| {
                    self.model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(node.size, |runtime| runtime.measured)
                })
                .collect();
            self.scene.edge_kinds = self
                .model
                .edges
                .iter()
                .map(|edge| EdgeKind::from_type(&edge.edge_type))
                .collect();
            self.scene.edge_ids = self.model.edges.iter().map(|edge| edge.id).collect();
            self.scene.edge_markers = self
                .model
                .edges
                .iter()
                .map(|edge| (edge.marker_start.is_some(), edge.marker_end.is_some()))
                .collect();
        }

        let motion = (membership, revisions.nodes, revisions.edges);
        if self.scene.motion != Some(motion) {
            self.scene.motion = Some(motion);
            self.scene.positions = self
                .model
                .nodes
                .iter()
                .map(|node| self.model.store.node_center_absolute(node))
                .collect();
            let positions = self.scene.positions.clone();
            let kinds = self.scene.edge_kinds.clone();
            let mut cache = std::mem::take(&mut self.edge_geometry_cache);
            let mut geometries = Vec::with_capacity(self.model.edges.len());
            for (index, (edge, layout)) in self
                .model
                .edges
                .iter()
                .zip(self.layout_edges.iter())
                .enumerate()
            {
                let kind = kinds.get(index).copied().unwrap_or(EdgeKind::Straight);
                if kind == EdgeKind::Straight {
                    geometries.push(None);
                    continue;
                }
                let stamp = (
                    *self.model.store.edge_revisions.get(&edge.id).unwrap_or(&0),
                    self.model
                        .store
                        .runtimes
                        .get(&edge.source)
                        .map_or(0, |runtime| runtime.revision),
                    self.model
                        .store
                        .runtimes
                        .get(&edge.target)
                        .map_or(0, |runtime| runtime.revision),
                );
                let a = positions[layout.source];
                let b = positions[layout.target];
                geometries.push(Some(cache.get_or_insert_with(edge.id, stamp, || {
                    match kind {
                        EdgeKind::Bezier => {
                            let (curve, _) = crate::connection::bezier_path(
                                a,
                                self.source_handle_position,
                                b,
                                self.target_handle_position,
                                0.25,
                            );
                            (0..=12)
                                .map(|index| {
                                    let t = index as f32 / 12.0;
                                    let u = 1.0 - t;
                                    WorldPoint::new(
                                        u * u * u * curve[0].x
                                            + 3.0 * u * u * t * curve[1].x
                                            + 3.0 * u * t * t * curve[2].x
                                            + t * t * t * curve[3].x,
                                        u * u * u * curve[0].y
                                            + 3.0 * u * u * t * curve[1].y
                                            + 3.0 * u * t * t * curve[2].y
                                            + t * t * t * curve[3].y,
                                    )
                                })
                                .collect()
                        }
                        EdgeKind::SmoothStep => {
                            crate::connection::smooth_step_path(
                                a,
                                self.source_handle_position,
                                b,
                                self.target_handle_position,
                                20.0,
                            )
                            .0
                        }
                        EdgeKind::Straight | EdgeKind::Custom => vec![a, b],
                    }
                })));
            }
            self.edge_geometry_cache = cache;
            self.scene.edge_geometries = geometries.into();
        }

        // Custom renderers are handed the whole node or edge, selection flag
        // included, so a selection change can repaint them even when nothing
        // was added or moved.
        let appearance = AppearanceRevision {
            membership,
            node_specs: revisions.node_specs,
            edge_specs: revisions.edge_specs,
            selection: revisions.selection,
            style: self.style_revision,
            zoom_bits: zoom.to_bits(),
        };
        if self.scene.appearance != Some(appearance) {
            self.scene.appearance = Some(appearance);
            self.scene.node_appearances = self
                .model
                .nodes
                .iter()
                .map(|node| renderer.node_appearance(node, zoom))
                .collect();
            self.scene.edge_appearances = self
                .model
                .edges
                .iter()
                .map(|edge| renderer.edge_appearance(edge))
                .collect();
        }

        let selection = (membership, revisions.selection);
        if self.scene.selection != Some(selection) {
            self.scene.selection = Some(selection);
            self.scene.selected = self
                .model
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| self.model.store.node_selected(node).then_some(index))
                .collect();
            let mut node_order = (0..self.model.nodes.len()).collect::<Vec<_>>();
            node_order.sort_by_key(|&index| {
                let node = &self.model.nodes[index];
                (
                    self.model
                        .store
                        .runtimes
                        .get(&node.id)
                        .map_or(0, |runtime| runtime.z),
                    index,
                )
            });
            self.scene.node_order = node_order.into();
            self.scene.selected_edges = self
                .model
                .edges
                .iter()
                .zip(self.layout_edges.iter())
                .filter_map(|(edge, layout)| {
                    self.model
                        .store
                        .edge_selected(edge)
                        .then_some((edge.id, *layout))
                })
                .collect();
        }
    }

    pub fn step_layout(&mut self) -> LayoutStatus {
        self.layout_positions.clear();
        self.layout_positions.extend(
            self.model
                .nodes
                .iter()
                .map(|node| self.model.store.node_position_absolute(node)),
        );
        if !self.layout_initialized {
            self.layout
                .initialize(&self.layout_positions, &self.layout_edges);
            self.layout_initialized = true;
        }
        let status = if self.layout.use_frame_budget() {
            step_with_budget(
                self.layout.as_mut(),
                &mut self.layout_positions,
                &self.layout_edges,
                self.layout_options.frame_budget,
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
        // Parents precede their children (enforced by graph validation). Keep
        // the simulation global, then serialize each result in its parent's
        // freshly computed coordinate system.
        let mut origins = std::collections::HashMap::new();
        let measured = self
            .model
            .nodes
            .iter()
            .map(|node| self.model.store.runtimes[&node.id].measured)
            .collect::<Vec<_>>();
        for ((node, absolute), size) in self
            .model
            .nodes
            .iter_mut()
            .zip(&self.layout_positions)
            .zip(measured)
        {
            let parent_origin = node
                .parent_id
                .and_then(|id| origins.get(&id).copied())
                .unwrap_or(WorldPoint::ZERO);
            node.position =
                WorldPoint::new(absolute.x - parent_origin.x, absolute.y - parent_origin.y);
            origins.insert(
                node.id,
                WorldPoint::new(
                    absolute.x - size.width * node.origin.x,
                    absolute.y - size.height * node.origin.y,
                ),
            );
        }
        self.model
            .store
            .sync_positions_from_specs(&self.model.nodes);
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

const RESIZE_DIRECTIONS: [crate::ResizeDirection; 8] = [
    crate::ResizeDirection::NorthWest,
    crate::ResizeDirection::North,
    crate::ResizeDirection::NorthEast,
    crate::ResizeDirection::East,
    crate::ResizeDirection::SouthEast,
    crate::ResizeDirection::South,
    crate::ResizeDirection::SouthWest,
    crate::ResizeDirection::West,
];

fn resize_handle_position(
    center: Point<Pixels>,
    node_size: crate::WorldSize,
    zoom: f32,
    direction: crate::ResizeDirection,
) -> Point<Pixels> {
    let x = match direction {
        crate::ResizeDirection::NorthWest
        | crate::ResizeDirection::West
        | crate::ResizeDirection::SouthWest => -1.0,
        crate::ResizeDirection::NorthEast
        | crate::ResizeDirection::East
        | crate::ResizeDirection::SouthEast => 1.0,
        _ => 0.0,
    };
    let y = match direction {
        crate::ResizeDirection::NorthWest
        | crate::ResizeDirection::North
        | crate::ResizeDirection::NorthEast => -1.0,
        crate::ResizeDirection::SouthWest
        | crate::ResizeDirection::South
        | crate::ResizeDirection::SouthEast => 1.0,
        _ => 0.0,
    };
    point(
        center.x + px(x * node_size.width * 0.5 * zoom),
        center.y + px(y * node_size.height * 0.5 * zoom),
    )
}

fn connection_handle_position(
    center: Point<Pixels>,
    radius_pixels: f32,
    kind: HandleKind,
    target_position: Position,
    source_position: Position,
    zoom: f32,
) -> Point<Pixels> {
    let offset = px(radius_pixels + CONNECTION_HANDLE_GAP_WORLD * zoom);
    match if kind == HandleKind::Target {
        target_position
    } else {
        source_position
    } {
        Position::Left => point(center.x - offset, center.y),
        Position::Top => point(center.x, center.y - offset),
        Position::Right => point(center.x + offset, center.y),
        Position::Bottom => point(center.x, center.y + offset),
    }
}

fn world_bounds(nodes: &[Node], store: &EditorStore) -> Option<WorldBounds> {
    let mut visible = nodes.iter().filter(|node| !node.hidden);
    let first = visible.next()?;
    let first = store.runtimes.get(&first.id)?.bounds();
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        first.origin.x,
        first.origin.x + first.size.width,
        first.origin.y,
        first.origin.y + first.size.height,
    );
    for node in visible {
        let bounds = store.runtimes.get(&node.id)?.bounds();
        min_x = min_x.min(bounds.origin.x);
        max_x = max_x.max(bounds.origin.x + bounds.size.width);
        min_y = min_y.min(bounds.origin.y);
        max_y = max_y.max(bounds.origin.y + bounds.size.height);
    }
    Some(WorldBounds::new(
        WorldPoint::new(min_x, min_y),
        crate::WorldSize::new(max_x - min_x, max_y - min_y),
    ))
}

impl Render for Graph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let data_updates = self.data_api.take_pending();
        if !data_updates.is_empty() {
            let changes = data_updates
                .into_iter()
                .filter_map(|(id, patch)| {
                    let mut node = self.model.node(id)?.clone();
                    node.metadata.extend(patch);
                    Some(NodeChange::Replace { id, item: node })
                })
                .collect::<Vec<_>>();
            self.model
                .emit_nodes(changes)
                .expect("graph data API produced valid metadata-only node updates");
            self.flush();
        }
        let revisions = self.model.store.dirty.revisions;
        let data_api_revision = (
            revisions.membership,
            revisions.node_specs,
            revisions.edge_specs,
            revisions.nodes,
            revisions.selection,
        );
        if self.data_api.has_external_consumer() && self.data_api_sync_revision != data_api_revision
        {
            self.data_api.sync(&self.model.nodes, &self.model.edges);
            self.data_api_sync_revision = data_api_revision;
        }
        self.sync();
        if self.fit_on_load_pending {
            self.fit_to_view(window.viewport_size(), px(40.0));
            self.fit_on_load_pending = false;
        }
        let viewport = self.viewport;
        let renderer = self.renderer.clone();
        let style = renderer.style().clone();
        let visible_nodes = if self.only_render_visible_elements {
            let size = window.viewport_size();
            let a = self.viewport.screen_to_world(point(px(0.0), px(0.0)));
            let b = self
                .viewport
                .screen_to_world(point(size.width, size.height));
            Some(Rc::new(
                self.model
                    .store
                    .visibility
                    .visible(WorldBounds::new(
                        WorldPoint::new(a.x.min(b.x), a.y.min(b.y)),
                        crate::WorldSize::new((a.x - b.x).abs(), (a.y - b.y).abs()),
                    ))
                    .collect::<std::collections::HashSet<_>>(),
            ))
        } else {
            None
        };
        self.refresh_scene(&renderer, viewport.zoom());
        let hidden = self.scene.hidden.clone();
        let node_ids = self.scene.node_ids.clone();
        let positions = self.scene.positions.clone();
        let node_sizes = self.scene.node_sizes.clone();
        let node_appearances = self.scene.node_appearances.clone();
        let edge_kinds = self.scene.edge_kinds.clone();
        let edge_ids = self.scene.edge_ids.clone();
        let edge_appearances = self.scene.edge_appearances.clone();
        let edge_markers = self.scene.edge_markers.clone();
        let edge_geometries = self.scene.edge_geometries.clone();
        let selected = self.scene.selected.clone();
        let show_handles = self.show_handles;
        let target_handle_position = self.target_handle_position;
        let source_handle_position = self.source_handle_position;
        let show_resize_handles = self.show_resize_handles;
        let resize_directions = self
            .model
            .nodes
            .iter()
            .map(|node| node.resize_directions.clone())
            .collect::<Vec<_>>();
        let show_resize_controls = self
            .model
            .nodes
            .iter()
            .map(|node| node.show_resize_controls)
            .collect::<Vec<_>>();
        let resize_controls_always_visible = self
            .model
            .nodes
            .iter()
            .map(|node| node.resize_controls_always_visible)
            .collect::<Vec<_>>();
        let resize_control_colors = self
            .model
            .nodes
            .iter()
            .map(|node| node.resize_control_color)
            .collect::<Vec<_>>();
        let selected_edges = self.scene.selected_edges.clone();
        let edges = self.layout_edges.clone();
        let edge_stride = renderer.interactive_edge_stride(edges.len(), self.playing);
        let reconnecting_edge = reconnecting_edge_id(&self.connection.state);
        let temporary_edge_preview = self.temporary_edge_preview;
        let default_node = NodeAppearance {
            color: style.node_color,
            radius_pixels: renderer.node_radius_pixels(viewport.zoom()),
            shape: NodeShape::Square,
        };
        let default_edge = EdgeAppearance {
            color: style.edge_color,
            width_pixels: style.edge_width_pixels,
        };
        let marquee = self.selection_start.zip(self.selection_current);
        let connection_line = match &self.connection.state {
            ConnectionState::Connecting {
                from,
                pointer,
                valid,
                ..
            } => self.model.node(from.node).map(|node| {
                let center = self.viewport.world_to_screen(self.node_center(node));
                let origin = self.viewport.screen_to_world(connection_handle_position(
                    center,
                    self.renderer
                        .node_appearance(node, self.viewport.zoom())
                        .radius_pixels,
                    from.kind,
                    self.target_handle_position,
                    self.source_handle_position,
                    self.viewport.zoom(),
                ));
                (origin, *pointer, *valid)
            }),
            _ => None,
        };

        let revisions = self.model.store.dirty.revisions;
        let viewport_size = window.viewport_size();
        let content_revision = (
            revisions.membership,
            revisions.node_specs,
            revisions.nodes,
            revisions.selection,
            revisions.viewport,
            viewport.zoom().to_bits(),
            (viewport_size.width / px(1.0)).to_bits(),
            (viewport_size.height / px(1.0)).to_bits(),
        );
        let mut uncached_node_contents = Vec::new();
        for index in self.scene.node_order.iter().copied() {
            let node = &self.model.nodes[index];
            if hidden[index]
                || visible_nodes
                    .as_ref()
                    .is_some_and(|visible| !visible.contains(&node.id))
            {
                continue;
            }
            let Some((content_renderer, cached)) = renderer.node_content_renderer(node) else {
                continue;
            };
            if cached {
                continue;
            }
            let center = viewport.world_to_screen(positions[index]);
            let width = px((node_sizes[index].width * viewport.zoom()).max(1.0));
            let height = px((node_sizes[index].height * viewport.zoom()).max(1.0));
            uncached_node_contents.push(
                div()
                    .absolute()
                    .left(center.x - width * 0.5)
                    .top(center.y - height * 0.5)
                    .w(width)
                    .h(height)
                    .child(content_renderer.render(node, viewport.zoom())),
            );
        }
        if self.node_content_layer_revision != Some(content_revision) {
            self.node_content_cache
                .retain(|id, _| self.model.store.node_lookup.contains_key(id));
            let mut items = Vec::new();
            for index in self.scene.node_order.iter().copied() {
                let node = &self.model.nodes[index];
                if hidden[index]
                    || visible_nodes
                        .as_ref()
                        .is_some_and(|visible| !visible.contains(&node.id))
                {
                    continue;
                }
                let Some((content_renderer, cached)) = renderer.node_content_renderer(node) else {
                    continue;
                };
                if !cached {
                    continue;
                }
                let content = if let Some(content) = self.node_content_cache.get(&node.id).cloned()
                {
                    let node = node.clone();
                    cx.update_entity(&content, |cached, cx| {
                        if cached.node != node
                            || cached.zoom.to_bits() != viewport.zoom().to_bits()
                            || !Arc::ptr_eq(&cached.renderer, &content_renderer)
                        {
                            cached.node = node;
                            cached.zoom = viewport.zoom();
                            cached.renderer = content_renderer;
                            cx.notify();
                        }
                    });
                    content
                } else {
                    let content = cx.new(|_| CachedNodeContent {
                        renderer: content_renderer,
                        node: node.clone(),
                        zoom: viewport.zoom(),
                    });
                    self.node_content_cache.insert(node.id, content.clone());
                    content
                };
                items.push(NodeContentItem {
                    center: viewport.world_to_screen(positions[index]),
                    size: size(
                        px((node_sizes[index].width * viewport.zoom()).max(1.0)),
                        px((node_sizes[index].height * viewport.zoom()).max(1.0)),
                    ),
                    content,
                });
            }
            cx.update_entity(&self.node_content_layer, |layer, cx| {
                layer.items = items;
                cx.notify();
            });
            self.node_content_layer_revision = Some(content_revision);
        }
        let node_content_layer = self.node_content_layer.clone();
        if self.data_api.has_pending() {
            cx.notify();
        }

        let graph_canvas = canvas(
            |_bounds, _window, _cx| (),
            move |bounds, _, window, _cx| {
                let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                let mut edge_path = Path::new(point(px(0.0), px(0.0)));
                for (edge_index, edge) in edges.iter().enumerate().step_by(edge_stride) {
                    if reconnecting_edge == Some(edge_ids[edge_index]) {
                        continue;
                    }
                    if hidden[edge.source] || hidden[edge.target] {
                        continue;
                    }
                    if visible_nodes.as_ref().is_some_and(|visible| {
                        !visible.contains(&node_ids[edge.source])
                            && !visible.contains(&node_ids[edge.target])
                    }) {
                        continue;
                    }
                    if edge_appearances[edge_index] != default_edge
                        || edge_kinds[edge_index] != EdgeKind::Straight
                    {
                        continue;
                    }
                    let p1 = viewport.world_to_screen(positions[edge.source]);
                    let p2 = viewport.world_to_screen(positions[edge.target]);
                    if (p1.x < bounds.left() && p2.x < bounds.left())
                        || (p1.x > bounds.right() && p2.x > bounds.right())
                        || (p1.y < bounds.top() && p2.y < bounds.top())
                        || (p1.y > bounds.bottom() && p2.y > bounds.bottom())
                    {
                        continue;
                    }
                    let direction = point(p2.x - p1.x, p2.y - p1.y);
                    let length = direction.magnitude() as f32;
                    if length <= 0.0001 {
                        continue;
                    }
                    let normal = point(-direction.y, direction.x)
                        * (style.edge_width_pixels.max(0.25) / length);
                    let p1a = point(p1.x + normal.x, p1.y + normal.y);
                    let p1b = point(p1.x - normal.x, p1.y - normal.y);
                    let p2a = point(p2.x + normal.x, p2.y + normal.y);
                    let p2b = point(p2.x - normal.x, p2.y - normal.y);
                    edge_path.push_triangle((p1a, p1b, p2a), st);
                    edge_path.push_triangle((p2a, p1b, p2b), st);
                }
                window.paint_path(edge_path, rgba((style.edge_color << 8) | 0x30));

                if let Some((from, to)) = temporary_edge_preview {
                    let from = viewport.world_to_screen(from);
                    let to = viewport.world_to_screen(to);
                    let delta = point(to.x - from.x, to.y - from.y);
                    let length = delta.magnitude() as f32;
                    let dots = (length / 10.0).floor().max(2.0) as usize;
                    for index in 0..=dots {
                        let t = index as f32 / dots as f32;
                        let center = point(from.x + delta.x * t, from.y + delta.y * t);
                        window.paint_quad(fill(
                            Bounds::centered_at(center, size(px(3.0), px(3.0))),
                            rgba(0x7f8792d9),
                        ));
                    }
                }

                for (edge_index, edge) in edges.iter().enumerate().step_by(edge_stride) {
                    if reconnecting_edge == Some(edge_ids[edge_index]) {
                        continue;
                    }
                    if hidden[edge.source] || hidden[edge.target] {
                        continue;
                    }
                    if visible_nodes.as_ref().is_some_and(|visible| {
                        !visible.contains(&node_ids[edge.source])
                            && !visible.contains(&node_ids[edge.target])
                    }) {
                        continue;
                    }
                    let appearance = edge_appearances[edge_index];
                    let kind = edge_kinds[edge_index];
                    if appearance == default_edge && kind == EdgeKind::Straight {
                        continue;
                    }
                    // Straight edges carry no cached geometry; a custom
                    // appearance on one still has to be drawn here, from its
                    // endpoints.
                    let straight;
                    let world_points = match &edge_geometries[edge_index] {
                        Some(points) => points,
                        None => {
                            straight = [positions[edge.source], positions[edge.target]];
                            &straight[..]
                        }
                    };
                    let mut path = Path::new(viewport.world_to_screen(world_points[0]));
                    for pair in world_points.windows(2) {
                        let p1 = viewport.world_to_screen(pair[0]);
                        let p2 = viewport.world_to_screen(pair[1]);
                        let direction = point(p2.x - p1.x, p2.y - p1.y);
                        let length = direction.magnitude() as f32;
                        if length <= 0.0001 {
                            continue;
                        }
                        let normal = point(-direction.y, direction.x)
                            * (appearance.width_pixels.max(0.25) / length);
                        path.push_triangle(
                            (
                                point(p1.x + normal.x, p1.y + normal.y),
                                point(p1.x - normal.x, p1.y - normal.y),
                                point(p2.x + normal.x, p2.y + normal.y),
                            ),
                            st,
                        );
                        path.push_triangle(
                            (
                                point(p2.x + normal.x, p2.y + normal.y),
                                point(p1.x - normal.x, p1.y - normal.y),
                                point(p2.x - normal.x, p2.y - normal.y),
                            ),
                            st,
                        );
                    }
                    window.paint_path(path, rgba((appearance.color << 8) | 0xff));
                }

                if !selected_edges.is_empty() {
                    let mut path = Path::new(point(px(0.0), px(0.0)));
                    for (edge_id, edge) in selected_edges.iter() {
                        if reconnecting_edge == Some(*edge_id) {
                            continue;
                        }
                        if hidden[edge.source] || hidden[edge.target] {
                            continue;
                        }
                        if visible_nodes.as_ref().is_some_and(|visible| {
                            !visible.contains(&node_ids[edge.source])
                                && !visible.contains(&node_ids[edge.target])
                        }) {
                            continue;
                        }
                        let Some(edge_index) = edge_ids.iter().position(|id| id == edge_id) else {
                            continue;
                        };
                        let straight;
                        let world_points = match &edge_geometries[edge_index] {
                            Some(points) => points.as_slice(),
                            None => {
                                straight = [positions[edge.source], positions[edge.target]];
                                &straight[..]
                            }
                        };
                        if world_points.len() < 2 {
                            continue;
                        }
                        for pair in world_points.windows(2) {
                            let p1 = viewport.world_to_screen(pair[0]);
                            let p2 = viewport.world_to_screen(pair[1]);
                            let direction = point(p2.x - p1.x, p2.y - p1.y);
                            let length = direction.magnitude() as f32;
                            if length <= 0.0001 {
                                continue;
                            }
                            let normal = point(-direction.y, direction.x) * (2.0 / length);
                            path.push_triangle(
                                (
                                    point(p1.x + normal.x, p1.y + normal.y),
                                    point(p1.x - normal.x, p1.y - normal.y),
                                    point(p2.x + normal.x, p2.y + normal.y),
                                ),
                                st,
                            );
                            path.push_triangle(
                                (
                                    point(p2.x + normal.x, p2.y + normal.y),
                                    point(p1.x - normal.x, p1.y - normal.y),
                                    point(p2.x - normal.x, p2.y - normal.y),
                                ),
                                st,
                            );
                        }
                        let endpoint_size = px(RECONNECT_HANDLE_SIZE_WORLD * viewport.zoom());
                        let endpoints = [
                            viewport.world_to_screen(world_points[0]),
                            viewport.world_to_screen(world_points[world_points.len() - 1]),
                        ];
                        for endpoint in endpoints {
                            window.paint_quad(fill(
                                Bounds::centered_at(endpoint, size(endpoint_size, endpoint_size)),
                                rgb(0xffffff),
                            ));
                            window.paint_quad(outline(
                                Bounds::centered_at(endpoint, size(endpoint_size, endpoint_size)),
                                rgb(style.selection_color),
                                BorderStyle::default(),
                            ));
                        }
                    }
                    window.paint_path(path, rgb(style.selection_color));
                }

                let mut marker_path = Path::new(point(px(0.0), px(0.0)));
                for (index, edge) in edges.iter().enumerate() {
                    if reconnecting_edge == Some(edge_ids[index]) {
                        continue;
                    }
                    if hidden[edge.source] || hidden[edge.target] {
                        continue;
                    }
                    if visible_nodes.as_ref().is_some_and(|visible| {
                        !visible.contains(&node_ids[edge.source])
                            && !visible.contains(&node_ids[edge.target])
                    }) {
                        continue;
                    }
                    let (start_marker, end_marker) = edge_markers[index];
                    if !start_marker && !end_marker {
                        continue;
                    }
                    let a = viewport.world_to_screen(positions[edge.source]);
                    let b = viewport.world_to_screen(positions[edge.target]);
                    let dx = (b.x - a.x) / px(1.0);
                    let dy = (b.y - a.y) / px(1.0);
                    let length = (dx * dx + dy * dy).sqrt();
                    if length <= 0.0001 {
                        continue;
                    }
                    let ux = dx / length;
                    let uy = dy / length;
                    let normal = point(px(-uy * 4.0), px(ux * 4.0));
                    if end_marker {
                        let base = point(b.x - px(ux * 9.0), b.y - px(uy * 9.0));
                        marker_path.push_triangle(
                            (
                                b,
                                point(base.x + normal.x, base.y + normal.y),
                                point(base.x - normal.x, base.y - normal.y),
                            ),
                            st,
                        )
                    }
                    if start_marker {
                        let base = point(a.x + px(ux * 9.0), a.y + px(uy * 9.0));
                        marker_path.push_triangle(
                            (
                                a,
                                point(base.x + normal.x, base.y + normal.y),
                                point(base.x - normal.x, base.y - normal.y),
                            ),
                            st,
                        )
                    }
                }
                window.paint_path(marker_path, rgb(style.edge_color));

                if let Some((a, b, valid)) = connection_line {
                    let p1 = viewport.world_to_screen(a);
                    let p2 = viewport.world_to_screen(b);
                    let direction = point(p2.x - p1.x, p2.y - p1.y);
                    let length = direction.magnitude() as f32;
                    if length > 0.0001 {
                        let normal = point(-direction.y, direction.x) * (1.5 / length);
                        let mut path = Path::new(p1);
                        path.push_triangle(
                            (
                                point(p1.x + normal.x, p1.y + normal.y),
                                point(p1.x - normal.x, p1.y - normal.y),
                                point(p2.x + normal.x, p2.y + normal.y),
                            ),
                            st,
                        );
                        path.push_triangle(
                            (
                                point(p2.x + normal.x, p2.y + normal.y),
                                point(p1.x - normal.x, p1.y - normal.y),
                                point(p2.x - normal.x, p2.y - normal.y),
                            ),
                            st,
                        );
                        window.paint_path(
                            path,
                            rgb(if valid == Some(false) {
                                0xd14343
                            } else {
                                0x1e90ff
                            }),
                        );
                    }
                }

                let radius = px(default_node.radius_pixels);
                let mut node_path = Path::new(point(px(0.0), px(0.0)));
                for (index, position) in positions.iter().enumerate() {
                    if hidden[index] {
                        continue;
                    }
                    if visible_nodes
                        .as_ref()
                        .is_some_and(|visible| !visible.contains(&node_ids[index]))
                    {
                        continue;
                    }
                    if node_appearances[index] != default_node {
                        continue;
                    }
                    let center = viewport.world_to_screen(*position);
                    if !bounds.contains(&center) {
                        continue;
                    }
                    let a = point(center.x - radius, center.y - radius);
                    let b = point(center.x + radius, center.y - radius);
                    let c = point(center.x + radius, center.y + radius);
                    let d = point(center.x - radius, center.y + radius);
                    node_path.push_triangle((a, b, c), st);
                    node_path.push_triangle((a, c, d), st);
                }
                window.paint_path(node_path, rgb(style.node_color));

                for (index, position) in positions.iter().enumerate() {
                    if hidden[index] {
                        continue;
                    }
                    if visible_nodes
                        .as_ref()
                        .is_some_and(|visible| !visible.contains(&node_ids[index]))
                    {
                        continue;
                    }
                    let appearance = node_appearances[index];
                    if appearance == default_node {
                        continue;
                    }
                    let center = viewport.world_to_screen(*position);
                    if !bounds.contains(&center) {
                        continue;
                    }
                    let r = px(appearance.radius_pixels);
                    let mut path = Path::new(center);
                    match appearance.shape {
                        NodeShape::None => {}
                        NodeShape::Square => {
                            let a = point(center.x - r, center.y - r);
                            let b = point(center.x + r, center.y - r);
                            let c = point(center.x + r, center.y + r);
                            let d = point(center.x - r, center.y + r);
                            path.push_triangle((a, b, c), st);
                            path.push_triangle((a, c, d), st)
                        }
                        NodeShape::Diamond => {
                            let a = point(center.x, center.y - r);
                            let b = point(center.x + r, center.y);
                            let c = point(center.x, center.y + r);
                            let d = point(center.x - r, center.y);
                            path.push_triangle((a, b, c), st);
                            path.push_triangle((a, c, d), st)
                        }
                    }
                    window.paint_path(path, rgb(appearance.color));
                }

                if show_handles {
                    let handle_size = px(CONNECTION_HANDLE_SIZE_WORLD * viewport.zoom());
                    let outer_half = handle_size * 0.5;
                    let inner_half = (outer_half - px(1.0)).max(px(0.0));
                    let mut handle_borders = Path::new(point(px(0.0), px(0.0)));
                    let mut handle_fills = Path::new(point(px(0.0), px(0.0)));
                    macro_rules! push_square {
                        ($path:expr, $center:expr, $half:expr) => {{
                            let center = $center;
                            let half = $half;
                            let a = point(center.x - half, center.y - half);
                            let b = point(center.x + half, center.y - half);
                            let c = point(center.x + half, center.y + half);
                            let d = point(center.x - half, center.y + half);
                            $path.push_triangle((a, b, c), st);
                            $path.push_triangle((a, c, d), st);
                        }};
                    }
                    for (index, position) in positions.iter().enumerate() {
                        if hidden[index] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible.contains(&node_ids[index]))
                        {
                            continue;
                        }
                        let center = viewport.world_to_screen(*position);
                        for kind in [HandleKind::Target, HandleKind::Source] {
                            let handle = connection_handle_position(
                                center,
                                node_appearances[index].radius_pixels,
                                kind,
                                target_handle_position,
                                source_handle_position,
                                viewport.zoom(),
                            );
                            push_square!(handle_borders, handle, outer_half);
                            if inner_half > px(0.0) {
                                push_square!(handle_fills, handle, inner_half);
                            }
                        }
                    }
                    window.paint_path(handle_borders, rgb(0x1e90ff));
                    window.paint_path(handle_fills, rgb(0xffffff));
                }

                for &index in selected.iter() {
                    if hidden[index] {
                        continue;
                    }
                    if visible_nodes
                        .as_ref()
                        .is_some_and(|visible| !visible.contains(&node_ids[index]))
                    {
                        continue;
                    }
                    let center = viewport.world_to_screen(positions[index]);
                    let selection_size = if matches!(node_appearances[index].shape, NodeShape::None)
                    {
                        size(
                            px((node_sizes[index].width * viewport.zoom()).max(1.0)),
                            px((node_sizes[index].height * viewport.zoom()).max(1.0)),
                        )
                    } else {
                        size(px(18.0), px(18.0))
                    };
                    window.paint_quad(outline(
                        Bounds::centered_at(center, selection_size),
                        rgb(style.selection_color),
                        BorderStyle::default(),
                    ));
                    if !show_handles {
                        let handle_size = px(CONNECTION_HANDLE_SIZE_WORLD * viewport.zoom());
                        for kind in [HandleKind::Target, HandleKind::Source] {
                            let handle = connection_handle_position(
                                center,
                                node_appearances[index].radius_pixels,
                                kind,
                                target_handle_position,
                                source_handle_position,
                                viewport.zoom(),
                            );
                            window.paint_quad(fill(
                                Bounds::centered_at(handle, size(handle_size, handle_size)),
                                rgb(0xffffff),
                            ));
                            window.paint_quad(outline(
                                Bounds::centered_at(handle, size(handle_size, handle_size)),
                                rgb(0x1e90ff),
                                BorderStyle::default(),
                            ));
                        }
                    }
                    if show_resize_handles && show_resize_controls[index] {
                        let resize_color =
                            resize_control_colors[index].unwrap_or(style.selection_color);
                        for direction in resize_directions[index]
                            .as_deref()
                            .unwrap_or(&RESIZE_DIRECTIONS)
                            .iter()
                            .copied()
                        {
                            let resize = resize_handle_position(
                                center,
                                node_sizes[index],
                                viewport.zoom(),
                                direction,
                            );
                            let handle_size = if matches!(
                                direction,
                                crate::ResizeDirection::NorthWest
                                    | crate::ResizeDirection::NorthEast
                                    | crate::ResizeDirection::SouthEast
                                    | crate::ResizeDirection::SouthWest
                            ) {
                                px(8.0)
                            } else {
                                px(7.0)
                            };
                            let corner = matches!(
                                direction,
                                crate::ResizeDirection::NorthWest
                                    | crate::ResizeDirection::NorthEast
                                    | crate::ResizeDirection::SouthEast
                                    | crate::ResizeDirection::SouthWest
                            );
                            window.paint_quad(fill(
                                Bounds::centered_at(resize, size(handle_size, handle_size)),
                                rgb(if corner { resize_color } else { 0xffffff }),
                            ));
                            window.paint_quad(outline(
                                Bounds::centered_at(resize, size(handle_size, handle_size)),
                                rgb(resize_color),
                                BorderStyle::default(),
                            ));
                        }
                    }
                }
                if show_resize_handles {
                    for (index, always_visible) in
                        resize_controls_always_visible.iter().copied().enumerate()
                    {
                        if !always_visible
                            || selected.contains(&index)
                            || hidden[index]
                            || !show_resize_controls[index]
                            || visible_nodes
                                .as_ref()
                                .is_some_and(|visible| !visible.contains(&node_ids[index]))
                        {
                            continue;
                        }
                        let center = viewport.world_to_screen(positions[index]);
                        let resize_color =
                            resize_control_colors[index].unwrap_or(style.selection_color);
                        for direction in resize_directions[index]
                            .as_deref()
                            .unwrap_or(&RESIZE_DIRECTIONS)
                            .iter()
                            .copied()
                        {
                            let resize = resize_handle_position(
                                center,
                                node_sizes[index],
                                viewport.zoom(),
                                direction,
                            );
                            let corner = matches!(
                                direction,
                                crate::ResizeDirection::NorthWest
                                    | crate::ResizeDirection::NorthEast
                                    | crate::ResizeDirection::SouthEast
                                    | crate::ResizeDirection::SouthWest
                            );
                            let handle_size = if corner { px(8.0) } else { px(7.0) };
                            window.paint_quad(fill(
                                Bounds::centered_at(resize, size(handle_size, handle_size)),
                                rgb(if corner { resize_color } else { 0xffffff }),
                            ));
                            window.paint_quad(outline(
                                Bounds::centered_at(resize, size(handle_size, handle_size)),
                                rgb(resize_color),
                                BorderStyle::default(),
                            ));
                        }
                    }
                }
                if let Some((start, end)) = marquee {
                    let origin = point(start.x.min(end.x), start.y.min(end.y));
                    let rectangle = Bounds::new(
                        origin,
                        size((start.x - end.x).abs(), (start.y - end.y).abs()),
                    );
                    window.paint_quad(outline(rectangle, rgba(0x1e90ffb0), BorderStyle::default()));
                }
            },
        )
        .absolute()
        .size_full();

        let graph_handle = cx.entity();
        let simulation = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _, window, cx| {
                let (playing, zooming) = cx.read_entity(&graph_handle, |graph: &Graph, _| {
                    (graph.playing, graph.smooth_zoom.is_some())
                });
                if playing || zooming {
                    window.request_animation_frame();
                    cx.update_entity(&graph_handle, |graph, cx| {
                        if graph.playing {
                            graph.step_layout();
                        }
                        graph.advance_smooth_zoom();
                        cx.notify();
                    });
                }
            },
        )
        .absolute()
        .size_full();

        let playing = self.playing;
        let edge_count = self.model.edges.len();
        let edge_budget = self.renderer.style().interactive_edge_budget;
        let controls = div()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .bg(rgb(0xf7f7f7))
            .border(px(1.0))
            .border_color(rgb(0xcccccc))
            .rounded(px(6.0))
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap_2()
            .cursor_default()
            .child(format!(
                "nodes: {}  edges: {}",
                self.model.nodes.len(),
                edge_count
            ))
            .child(if playing && edge_count > edge_budget {
                format!(
                    "interactive edge LOD: 1/{}",
                    edge_count.div_ceil(edge_budget)
                )
            } else {
                "full edge detail".to_string()
            })
            .child(format!("layout frame: {}", self.sim_tick))
            .child(self.announcement.clone());

        let play_button = div()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .size(px(28.0))
            .rounded_full()
            .bg(if playing {
                rgb(0x4CAF50)
            } else {
                rgb(0xeeeeee)
            })
            .border(px(1.0))
            .border_color(rgb(0xcccccc))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|graph, _: &MouseDownEvent, window, cx| {
                    if graph.playing {
                        graph.stop_layout();
                        graph.fit_to_view(window.viewport_size(), px(40.0));
                    } else {
                        graph.start_layout()
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );

        let canvas_cursor = if self.pan_drag_position.is_some() {
            CursorStyle::ClosedHand
        } else if self.pointer_over_handle {
            CursorStyle::Crosshair
        } else if self.pointer_over_graph_item {
            CursorStyle::Arrow
        } else {
            CursorStyle::OpenHand
        };
        let edge_labels = self
            .model
            .edges
            .iter()
            .filter(|edge| reconnecting_edge != Some(edge.id))
            .filter_map(|edge| {
                let label = edge.label.as_ref()?;
                let source = self
                    .model
                    .store
                    .node_center_absolute(self.model.node(edge.source)?);
                let target = self
                    .model
                    .store
                    .node_center_absolute(self.model.node(edge.target)?);
                let midpoint = self.viewport.world_to_screen(WorldPoint::new(
                    (source.x + target.x) * 0.5,
                    (source.y + target.y) * 0.5,
                ));
                Some(
                    div()
                        .absolute()
                        .left(midpoint.x)
                        .top(midpoint.y)
                        .px_1()
                        .bg(rgba(0xffffffe0))
                        .child(label.clone()),
                )
            })
            .collect::<Vec<_>>();

        div()
            .id("gpug-graph")
            .track_focus(&self.focus)
            .key_context("gpug-graph")
            .size_full()
            .bg(rgb(self.renderer.style().background))
            .cursor(canvas_cursor)
            .child(simulation)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseDownEvent, window, cx| {
                    graph.focus.focus(window);
                    if let Some((index, direction)) =
                        graph.resize_at_screen_position(event.position)
                    {
                        let node = &graph.model.nodes[index];
                        let mut control = crate::NodeResizeControl::new(node.id, direction);
                        control.begin(
                            graph.screen_to_world(event.position),
                            graph.model.store.runtimes[&node.id].bounds(),
                        );
                        graph.resize_node = Some((index, control));
                        graph.announce("Resizing node");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    // A handle is the more specific target when its hit box
                    // overlaps a selected edge endpoint. This keeps a drag
                    // from the node's source handle anchored to that node.
                    if let Some((key, _)) = graph
                        .handle_at_screen_position(event.position, false)
                        .or_else(|| graph.handle_at_screen_position(event.position, true))
                    {
                        let pointer = graph.screen_to_world(event.position);
                        graph.connection.arm(key.clone(), ConnectionIntent::Create);
                        graph.connection.begin(pointer);
                        graph.events.push(GraphEvent::ConnectStart {
                            from: key,
                            intent: ConnectionIntent::Create,
                        });
                        graph.model.store.dirty.mark_connection();
                        graph
                            .gestures
                            .claim(GestureOwner::Handle, Gesture::Connection);
                        graph.announce("Connection started");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    if let Some((key, intent)) = graph.reconnect_at_screen_position(event.position)
                    {
                        let pointer = graph.screen_to_world(event.position);
                        graph.connection.arm(key.clone(), intent.clone());
                        graph.connection.begin(pointer);
                        graph
                            .events
                            .push(GraphEvent::ConnectStart { from: key, intent });
                        graph.model.store.dirty.mark_connection();
                        graph
                            .gestures
                            .claim(GestureOwner::Handle, Gesture::Connection);
                        graph.announce("Reconnecting edge");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    let hit = graph.node_at_screen_position(event.position);
                    if let Some(index) = hit {
                        let id = graph.model.nodes[index].id;
                        graph
                            .model
                            .select_node(id, event.modifiers.shift, event.modifiers.shift);
                        if !graph.model.store.node_selected(&graph.model.nodes[index]) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                        if !graph.node_allows_drag_at_screen_position(
                            &graph.model.nodes[index],
                            event.position,
                        ) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                        let world = graph.screen_to_world(event.position);
                        graph.drag_nodes = Some(
                            graph
                                .model
                                .nodes
                                .iter()
                                .enumerate()
                                .filter(|(_, node)| {
                                    graph.model.store.node_selected(node) && node.draggable
                                })
                                .map(|(index, node)| {
                                    (index, {
                                        let absolute =
                                            graph.model.store.node_position_absolute(node);
                                        WorldPoint::new(world.x - absolute.x, world.y - absolute.y)
                                    })
                                })
                                .collect(),
                        );
                        let affected = graph
                            .drag_nodes
                            .as_ref()
                            .into_iter()
                            .flatten()
                            .map(|(index, _)| graph.model.nodes[*index].id)
                            .collect();
                        graph.pointer = Some(PointerController::begin(
                            ViewportPoint::new(
                                event.position.x / px(1.0),
                                event.position.y / px(1.0),
                            ),
                            graph.gestures.drag_threshold,
                            affected,
                        ));
                        let node = &graph.model.nodes[index];
                        graph.gestures.claim(
                            GestureOwner::NodeDrag,
                            Gesture::NodeDrag {
                                node: node.id,
                                pointer_offset: WorldPoint::ZERO,
                            },
                        );
                    } else if let Some(index) = graph.edge_index_at_screen_position(event.position)
                    {
                        let id = graph.model.edges[index].id;
                        graph
                            .model
                            .select_edge(id, event.modifiers.shift, event.modifiers.shift);
                    } else {
                        graph.smooth_zoom = None;
                        if event.modifiers.shift {
                            graph.selection_start = Some(event.position);
                            graph.selection_current = Some(event.position);
                            graph.gestures.claim(
                                GestureOwner::Marquee,
                                Gesture::Marquee {
                                    start: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                    current: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                },
                            );
                        } else {
                            graph.model.clear_selection();
                            graph.pan_drag_position = Some(event.position);
                            graph.pointer = Some(PointerController::begin(
                                ViewportPoint::new(
                                    event.position.x / px(1.0),
                                    event.position.y / px(1.0),
                                ),
                                graph.gestures.drag_threshold,
                                Vec::new(),
                            ));
                            graph.gestures.claim(
                                GestureOwner::Viewport,
                                Gesture::ViewportPan {
                                    previous: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                },
                            );
                        }
                    }
                    graph.flush();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|graph, event: &KeyDownEvent, _, cx| {
                if graph.handle_key(event) {
                    cx.stop_propagation();
                    cx.notify();
                }
                graph.flush();
            }))
            .on_mouse_move(cx.listener(|graph, event: &MouseMoveEvent, _, cx| {
                if let Some((index, control)) = graph.resize_node {
                    graph.pointer_over_handle = false;
                    let pointer = graph.screen_to_world(event.position);
                    if let Some(resized) = control.update(pointer) {
                        let id = graph.model.nodes[index].id;
                        graph.model.resize_node_from_bounds(id, resized, true);
                    }
                } else if matches!(graph.connection.state, ConnectionState::Connecting { .. }) {
                    let pointer = graph.screen_to_world(event.position);
                    let target_is_end = matches!(
                        graph.connection.state,
                        ConnectionState::Connecting {
                            ref from,
                            ..
                        } if from.kind == HandleKind::Source
                    );
                    let target = graph
                        .handle_at_screen_position(event.position, target_is_end)
                        .map(|(key, center)| graph.connection_handle(key, center));
                    graph.pointer_over_handle = target.is_some();
                    graph
                        .connection
                        .update(pointer, target.as_ref(), std::iter::empty());
                    graph.model.store.dirty.mark_connection();
                } else if let Some(items) = graph.drag_nodes.clone() {
                    graph.pointer_over_handle = false;
                    if let Some(pointer) = &mut graph.pointer {
                        if !pointer.update(ViewportPoint::new(
                            event.position.x / px(1.0),
                            event.position.y / px(1.0),
                        )) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                    }
                    let world = graph.screen_to_world(event.position);
                    let targets = items
                        .into_iter()
                        .map(|(index, offset)| {
                            (
                                graph.model.nodes[index].id,
                                WorldPoint::new(world.x - offset.x, world.y - offset.y),
                            )
                        })
                        .collect::<Vec<_>>();
                    graph.model.move_nodes(&targets, true);
                    graph.layout_initialized = false;
                } else if let Some(start) = graph.selection_start {
                    graph.pointer_over_handle = false;
                    graph.selection_current = Some(event.position);
                    let a = graph.screen_to_world(start);
                    let b = graph.screen_to_world(event.position);
                    let rect = WorldBounds::new(
                        WorldPoint::new(a.x.min(b.x), a.y.min(b.y)),
                        crate::WorldSize::new((a.x - b.x).abs(), (a.y - b.y).abs()),
                    );
                    graph
                        .model
                        .select_rect(rect, graph.selection_mode, event.modifiers.shift);
                } else if let Some(previous) = graph.pan_drag_position {
                    if let Some(pointer) = &mut graph.pointer {
                        if !pointer.update(ViewportPoint::new(
                            event.position.x / px(1.0),
                            event.position.y / px(1.0),
                        )) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                    }
                    graph.pan_by(point(
                        event.position.x - previous.x,
                        event.position.y - previous.y,
                    ));
                    graph.pan_drag_position = Some(event.position);
                    graph.pointer_over_graph_item = false;
                    graph.pointer_over_handle = false;
                } else {
                    graph.pointer_over_handle = graph.is_handle_at_screen_position(event.position);
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                }
                graph.flush();
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseUpEvent, _, cx| {
                    let owner = graph.gestures.owner();
                    if let Some((index, mut control)) = graph.resize_node.take() {
                        let node = graph.model.nodes[index].clone();
                        let pointer = graph.screen_to_world(event.position);
                        let bounds = control
                            .end(pointer)
                            .unwrap_or_else(|| graph.model.store.runtimes[&node.id].bounds());
                        graph.model.resize_node_from_bounds(node.id, bounds, false);
                        graph.announce("Node resize finished");
                    }
                    let pending = graph.connection.pending();
                    let mut connected = false;
                    if let Some((connection, intent)) = graph.connection.finish() {
                        graph.model.store.dirty.mark_connection();
                        if matches!(intent, ConnectionIntent::Create) {
                            let mut edge =
                                Edge::new(connection.source.node, connection.target.node)
                                    .with_id(graph.next_edge_id);
                            edge.source_handle = connection.source.id.as_deref().map(str::to_owned);
                            edge.target_handle = connection.target.id.as_deref().map(str::to_owned);
                            if graph.model.add_edge_with_id(edge.clone()) {
                                graph.next_edge_id = graph.next_edge_id.wrapping_add(1);
                                graph.events.push(GraphEvent::Connected(edge.clone()));
                                graph.announce("Edge connected");
                                connected = true;
                            }
                        } else {
                            let id = match intent {
                                ConnectionIntent::ReconnectSource(id)
                                | ConnectionIntent::ReconnectTarget(id) => id,
                                ConnectionIntent::Create => unreachable!(),
                            };
                            if graph.model.reconnect(id, intent, &connection) {
                                let edge = graph
                                    .model
                                    .edge(id)
                                    .expect("reconnected edge remains in the model")
                                    .clone();
                                graph.events.push(GraphEvent::Reconnected {
                                    id,
                                    edge: edge.clone(),
                                });
                                graph.announce("Edge reconnected");
                                connected = true;
                            }
                        }
                    } else {
                        graph.connection.cancel();
                        graph.model.store.dirty.mark_connection();
                    }
                    if let Some((from, intent, _)) = pending {
                        // The controller only keeps the pointer it was last
                        // moved to; the release position is the truthful drop
                        // point, and the two differ when the button comes up
                        // without an intervening move.
                        graph.events.push(GraphEvent::ConnectEnd {
                            from,
                            intent,
                            position: graph.screen_to_world(event.position),
                            connected,
                        });
                    }
                    if let Some(items) = graph.drag_nodes.take() {
                        let targets = items
                            .iter()
                            .map(|(index, _)| {
                                let node = &graph.model.nodes[*index];
                                (node.id, graph.model.store.node_position_absolute(node))
                            })
                            .collect::<Vec<_>>();
                        graph.model.move_nodes(&targets, false);
                        graph.announce("Node drag finished");
                    }
                    graph.pan_drag_position = None;
                    if let Some(pointer) = &mut graph.pointer {
                        pointer.end();
                    }
                    graph.pointer = None;
                    graph.selection_start = None;
                    graph.selection_current = None;
                    graph.gestures.finish();
                    if owner == Some(GestureOwner::Marquee) {
                        graph.announce("Marquee selection finished");
                    }
                    if owner == Some(GestureOwner::Viewport) {
                        graph
                            .events
                            .push(GraphEvent::ViewportChanged(graph.viewport));
                    }
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                    graph.pointer_over_handle = graph.is_handle_at_screen_position(event.position);
                    graph.flush();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|graph, event: &ScrollWheelEvent, _, cx| {
                let dy = event.delta.pixel_delta(px(16.0)).y;
                if dy != px(0.0) {
                    let steps = ((dy / px(16.0)).abs()).max(0.01);
                    // A gentle per-notch scale keeps mouse wheels smooth while
                    // still preserving proportional high-resolution trackpad input.
                    let factor = 1.04_f32.powf(steps);
                    let factor = if dy > px(0.0) { factor } else { factor.recip() };
                    graph.queue_smooth_zoom(factor, event.position);
                    cx.notify();
                }
            }))
            .child(graph_canvas)
            .child(node_content_layer)
            .children(uncached_node_contents)
            .children(edge_labels)
            .child(controls)
            .child(play_button)
    }
}

#[cfg(test)]
mod tests {
    use super::{reconnecting_edge_id, GraphDataApi};
    use crate::{
        ConnectionIntent, ConnectionState, Edge, EdgeId, HandleKey, HandleKind, Node, NodeId,
        WorldPoint,
    };

    #[test]
    fn only_an_edge_being_reconnected_is_hidden_from_painting() {
        let from = HandleKey {
            node: NodeId(1),
            id: None,
            kind: HandleKind::Source,
        };
        let reconnecting = ConnectionState::Connecting {
            from: from.clone(),
            to: None,
            pointer: WorldPoint::ZERO,
            valid: None,
            intent: ConnectionIntent::ReconnectTarget(EdgeId(7)),
        };
        let creating = ConnectionState::Connecting {
            from,
            to: None,
            pointer: WorldPoint::ZERO,
            valid: None,
            intent: ConnectionIntent::Create,
        };

        assert_eq!(reconnecting_edge_id(&reconnecting), Some(EdgeId(7)));
        assert_eq!(reconnecting_edge_id(&creating), None);
        assert_eq!(reconnecting_edge_id(&ConnectionState::Idle), None);
    }

    #[test]
    fn data_api_reads_live_connections_and_merges_node_data() {
        let mut source = Node::new(1_u64, WorldPoint::ZERO);
        source.metadata.insert("text".into(), "hello".into());
        let target = Node::new(2_u64, WorldPoint::ZERO);
        let edge = Edge::new(source.id, target.id).with_id(7_u64);
        let api = GraphDataApi::new();
        api.sync(&[source, target], &[edge]);

        let connections = api.node_connections(NodeId(2), HandleKind::Target);
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].source, NodeId(1));
        assert_eq!(
            api.nodes_data(connections.into_iter().map(|edge| edge.source))[0]
                .metadata
                .get("text")
                .map(String::as_str),
            Some("hello")
        );

        assert!(api.update_node_data(NodeId(1), [("text".into(), "updated".into())]));
        assert_eq!(
            api.node_data(NodeId(1)).unwrap().metadata["text"],
            "updated"
        );
        assert_eq!(api.take_pending().len(), 1);
    }
}

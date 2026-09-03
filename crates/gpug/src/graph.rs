use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{canvas, div, *};

use crate::connection::{ConnectionController, ConnectionIntent, ConnectionState};
use crate::coordinates::ViewportPoint;
use crate::coordinates::{Viewport, WorldBounds, WorldPoint};
use crate::data::{GraphData, GraphDataError, LayoutEdge};
use crate::edge::{Edge, EdgeMarker};
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
use crate::profile::{self, Counter, Phase};
use crate::renderer::GraphRenderer;
use crate::renderer::{EdgeAppearance, NodeAppearance, NodeShape};
use crate::style::GraphStyle;

#[derive(Clone, Copy)]
struct SmoothZoom {
    target: f32,
    anchor: Point<Pixels>,
}

fn local_to_window(point: Point<Pixels>, origin: Point<Pixels>) -> Point<Pixels> {
    gpui::point(point.x + origin.x, point.y + origin.y)
}

fn window_to_local(point: Point<Pixels>, origin: Point<Pixels>) -> Point<Pixels> {
    gpui::point(point.x - origin.x, point.y - origin.y)
}

fn segment_intersects_bounds(start: WorldPoint, end: WorldPoint, bounds: WorldBounds) -> bool {
    let min_x = bounds.origin.x;
    let max_x = min_x + bounds.size.width;
    let min_y = bounds.origin.y;
    let max_y = min_y + bounds.size.height;
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let mut entry = 0.0_f32;
    let mut exit = 1.0_f32;

    for (p, q) in [
        (-dx, start.x - min_x),
        (dx, max_x - start.x),
        (-dy, start.y - min_y),
        (dy, max_y - start.y),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return false;
            }
            continue;
        }
        let ratio = q / p;
        if p < 0.0 {
            entry = entry.max(ratio);
        } else {
            exit = exit.min(ratio);
        }
        if entry > exit {
            return false;
        }
    }
    true
}

#[cfg(test)]
fn reconnect_endpoint_hit(
    position: Point<Pixels>,
    endpoint: Point<Pixels>,
    toward: Point<Pixels>,
    edge_width_pixels: f32,
    max_length_pixels: f32,
) -> bool {
    reconnect_endpoint_hit_distance(
        position,
        endpoint,
        toward,
        edge_width_pixels,
        max_length_pixels,
    )
    .is_some()
}

fn reconnect_endpoint_hit_distance(
    position: Point<Pixels>,
    endpoint: Point<Pixels>,
    toward: Point<Pixels>,
    edge_width_pixels: f32,
    max_length_pixels: f32,
) -> Option<f32> {
    let dx = (toward.x - endpoint.x) / px(1.0);
    let dy = (toward.y - endpoint.y) / px(1.0);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 0.0001 {
        return None;
    }

    let ux = dx / length;
    let uy = dy / length;
    let rx = (position.x - endpoint.x) / px(1.0);
    let ry = (position.y - endpoint.y) / px(1.0);
    let along = rx * ux + ry * uy;
    let across = (rx * -uy + ry * ux).abs();
    let half_width = edge_width_pixels.max(0.0) * 0.5 + RECONNECT_HIT_PADDING_PIXELS;

    ((0.0..=length.min(max_length_pixels)).contains(&along) && across <= half_width)
        .then_some(along)
}

#[cfg(test)]
fn reconnect_path_end_hit(
    position: Point<Pixels>,
    path: &[Point<Pixels>],
    from_start: bool,
    edge_width_pixels: f32,
    zoom: f32,
) -> bool {
    reconnect_path_end_distance(position, path, from_start, edge_width_pixels, zoom).is_some()
}

fn reconnect_path_end_distance(
    position: Point<Pixels>,
    path: &[Point<Pixels>],
    from_start: bool,
    edge_width_pixels: f32,
    zoom: f32,
) -> Option<f32> {
    let mut remaining = reconnect_hit_length_pixels(zoom);
    let mut traversed = 0.0;
    let mut visit = |a: Point<Pixels>, b: Point<Pixels>| {
        let dx = (b.x - a.x) / px(1.0);
        let dy = (b.y - a.y) / px(1.0);
        let length = (dx * dx + dy * dy).sqrt();
        let hit = reconnect_endpoint_hit_distance(position, a, b, edge_width_pixels, remaining)
            .map(|distance| traversed + distance);
        remaining -= length.min(remaining);
        traversed += length;
        hit
    };

    if from_start {
        path.windows(2).find_map(|pair| visit(pair[0], pair[1]))
    } else {
        path.windows(2)
            .rev()
            .find_map(|pair| visit(pair[1], pair[0]))
    }
}

fn reconnect_hit_length_pixels(zoom: f32) -> f32 {
    RECONNECT_HIT_LENGTH_PIXELS * zoom.max(1.0)
}

fn connection_handle_contains(
    position: Point<Pixels>,
    center: Point<Pixels>,
    diameter: Pixels,
) -> bool {
    let dx = (position.x - center.x) / px(1.0);
    let dy = (position.y - center.y) / px(1.0);
    let radius = diameter * 0.5 / px(1.0);
    dx * dx + dy * dy <= radius * radius
}

struct CachedNodeContent {
    renderer: Arc<dyn crate::renderer::NodeContentRenderer>,
    node: Node,
    zoom: f32,
}

type NodeContentLayerRevision = (u64, u64, u64, u64, u64, u32, u32, u32, u32, u32);

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
/// Length of the invisible target extending inward along each end of a selected edge.
const RECONNECT_HIT_LENGTH_PIXELS: f32 = 2.0;
/// Extra target width on each side of the rendered edge stroke.
const RECONNECT_HIT_PADDING_PIXELS: f32 = 1.0;
/// On-screen node height, in pixels, below which a rounded body degrades to a
/// flat fill. Corner rounding and a one-pixel border are both sub-pixel below
/// this, so the shaped quad costs shader work that resolves to nothing.
const SHELL_LOD_MIN_PIXELS: f32 = 5.0;
const MULTI_SELECTION_PADDING_PIXELS: f32 = 6.0;

/// Edge type resolved once per membership change. Comparing a copyable tag per
/// frame beats re-reading and re-hashing every edge's type string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EdgeKind {
    Straight,
    Bezier,
    Step,
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
            "step" => Self::Step,
            "smoothstep" => Self::SmoothStep,
            _ => Self::Custom,
        }
    }
}

fn node_side_point(center: WorldPoint, size: crate::WorldSize, side: Position) -> WorldPoint {
    match side {
        Position::Left => WorldPoint::new(center.x - size.width * 0.5, center.y),
        Position::Right => WorldPoint::new(center.x + size.width * 0.5, center.y),
        Position::Top => WorldPoint::new(center.x, center.y - size.height * 0.5),
        Position::Bottom => WorldPoint::new(center.x, center.y + size.height * 0.5),
    }
}

fn connection_edge_world_position(
    center: WorldPoint,
    size: crate::WorldSize,
    side: Position,
) -> WorldPoint {
    let point = node_side_point(center, size, side);
    let offset = CONNECTION_HANDLE_GAP_WORLD + CONNECTION_HANDLE_SIZE_WORLD * 0.5;
    match side {
        Position::Left => WorldPoint::new(point.x - offset, point.y),
        Position::Right => WorldPoint::new(point.x + offset, point.y),
        Position::Top => WorldPoint::new(point.x, point.y - offset),
        Position::Bottom => WorldPoint::new(point.x, point.y + offset),
    }
}

fn connection_geometry_size(
    node_size: crate::WorldSize,
    appearance: NodeAppearance,
    zoom: f32,
) -> crate::WorldSize {
    match appearance.shape {
        NodeShape::Square | NodeShape::Diamond if zoom.is_finite() && zoom > 0.0 => {
            let diameter_world = appearance.radius_pixels.max(0.0) * 2.0 / zoom;
            crate::WorldSize::new(diameter_world, diameter_world)
        }
        NodeShape::None | NodeShape::Rect { .. } | NodeShape::Square | NodeShape::Diamond => {
            node_size
        }
    }
}

fn self_loop_path(source: WorldPoint, target: WorldPoint) -> Vec<WorldPoint> {
    let extent = ((source.y - target.y).abs() * 1.5).max(10.0);
    let c1 = WorldPoint::new(source.x + extent, source.y + extent * 0.5);
    let c2 = WorldPoint::new(target.x + extent, target.y - extent * 0.5);
    (0..=24)
        .map(|index| {
            let t = index as f32 / 24.0;
            let u = 1.0 - t;
            WorldPoint::new(
                source.x * (u * u * u)
                    + c1.x * (3.0 * u * u * t)
                    + c2.x * (3.0 * u * t * t)
                    + target.x * (t * t * t),
                source.y * (u * u * u)
                    + c1.y * (3.0 * u * u * t)
                    + c2.y * (3.0 * u * t * t)
                    + target.y * (t * t * t),
            )
        })
        .collect()
}

#[cfg(test)]
fn facing_sides(source: WorldPoint, target: WorldPoint) -> (Position, Position) {
    let dx = target.x - source.x;
    let dy = target.y - source.y;
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 {
            (Position::Right, Position::Left)
        } else {
            (Position::Left, Position::Right)
        }
    } else if dy >= 0.0 {
        (Position::Bottom, Position::Top)
    } else {
        (Position::Top, Position::Bottom)
    }
}

/// Per-frame paint inputs, grouped by what invalidates them. Each group carries
/// the revision stamp it was built at, so a frame that only pans the camera
/// reuses every array instead of rebuilding one entry per node and edge.
#[derive(Default)]
struct SceneCache {
    specs: Option<(u64, u64, u64)>,
    motion: Option<(u64, u64, u64, u32)>,
    appearance: Option<AppearanceRevision>,
    selection: Option<(u64, u64)>,
    hidden: Rc<[bool]>,
    node_ids: Rc<[NodeId]>,
    node_sizes: Rc<[crate::WorldSize]>,
    edge_kinds: Rc<[EdgeKind]>,
    /// Whether any edge needs sampled geometry, and whether any carries a
    /// label. Both let a frame skip a full pass over the edge list rather than
    /// walking it to discover there was nothing to do.
    any_curved_edges: bool,
    any_edge_labels: bool,
    edge_ids: Rc<[crate::EdgeId]>,
    edge_markers: Rc<[(Option<EdgeMarker>, Option<EdgeMarker>)]>,
    edge_appearances: Rc<[EdgeAppearance]>,
    /// Rebuilt on every frame that moves a node, so these are the two buffers
    /// that must never reallocate. `Rc::make_mut` writes in place while the
    /// previous frame's paint closure has been dropped, and copies only if it
    /// has not - which is the adaptive form of double buffering.
    positions: Rc<Vec<WorldPoint>>,
    edge_geometries: Rc<Vec<Option<Vec<WorldPoint>>>>,
    node_appearances: Rc<[NodeAppearance]>,
    selected: Rc<[usize]>,
    node_order: Rc<[usize]>,
    /// Edge index alongside its endpoints, so the selection pass indexes the
    /// scene columns directly instead of searching the id column per edge.
    selected_edges: Rc<[(usize, LayoutEdge)]>,
    resize_directions: Rc<[Option<Rc<[crate::ResizeDirection]>>]>,
    show_resize_controls: Rc<[bool]>,
    resize_controls_always_visible: Rc<[bool]>,
    resize_control_colors: Rc<[Option<u32>]>,
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

fn marker_triangle(
    tip: Point<Pixels>,
    previous: Point<Pixels>,
    edge_width: f32,
) -> Option<(Point<Pixels>, Point<Pixels>, Point<Pixels>)> {
    let dx = (tip.x - previous.x) / px(1.0);
    let dy = (tip.y - previous.y) / px(1.0);
    let direction_length = (dx * dx + dy * dy).sqrt();
    if direction_length <= 0.0001 {
        return None;
    }

    let ux = dx / direction_length;
    let uy = dy / direction_length;
    let length = 9.0 + edge_width.max(0.0);
    let half_width = length * 0.6;
    let base = point(tip.x - px(ux * length), tip.y - px(uy * length));
    let normal = point(px(-uy * half_width), px(ux * half_width));
    Some((
        tip,
        point(base.x + normal.x, base.y + normal.y),
        point(base.x - normal.x, base.y - normal.y),
    ))
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
    auto_pan: bool,
    auto_pan_speed: f32,
    auto_pan_margin: f32,
    ownership: GraphOwnership,
    data_api: GraphDataApi,
    connection_validator: Option<crate::ConnectionValidator>,
    default_edge_marker_start: Option<EdgeMarker>,
    default_edge_marker_end: Option<EdgeMarker>,
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
            show_handles: true,
            target_handle_position: Position::Left,
            source_handle_position: Position::Right,
            show_resize_handles: false,
            // Culling is a pure win: the visible set is a conservative
            // superset of what the camera can show, so a graph looks the same
            // with it on and stops paying for what is off screen.
            only_render_visible_elements: true,
            selection_mode: SelectionMode::Partial,
            snap_grid: None,
            ownership: GraphOwnership::Internal,
            auto_pan: true,
            auto_pan_speed: Graph::DEFAULT_AUTO_PAN_SPEED,
            auto_pan_margin: Graph::DEFAULT_AUTO_PAN_MARGIN,
            data_api: GraphDataApi::default(),
            connection_validator: None,
            default_edge_marker_start: None,
            default_edge_marker_end: Some(EdgeMarker::ArrowClosed),
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

    /// Validates a proposed connection before it is added to the graph.
    pub fn connection_validator(
        mut self,
        validator: impl Fn(&crate::Connection) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.connection_validator = Some(Arc::new(validator));
        self
    }

    /// Sets the markers used by edges created through handle-drag gestures.
    ///
    /// This does not modify edges supplied in [`GraphData`]. By default,
    /// gesture-created edges use no start marker and a closed-arrow end marker.
    pub fn default_edge_markers(
        mut self,
        start: Option<EdgeMarker>,
        end: Option<EdgeMarker>,
    ) -> Self {
        self.default_edge_marker_start = start;
        self.default_edge_marker_end = end;
        self
    }

    pub fn viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = viewport;
        self
    }

    /// Sets the maximum viewport scale in screen pixels per world unit.
    ///
    /// The default is [`Viewport::MAX_ZOOM`] (60×). Invalid values are ignored.
    pub fn max_zoom(mut self, max_zoom: f32) -> Self {
        self.viewport.set_max_zoom(max_zoom);
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

    /// Controls whether connection handles remain visible when their node is
    /// not selected. Handles are visible by default.
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
    /// Scratch column for hierarchical write-back, retained so a layout frame
    /// allocates nothing.
    layout_origins: Vec<WorldPoint>,
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
    pointer_over_reconnect: bool,
    smooth_zoom: Option<SmoothZoom>,
    gestures: GestureRouter,
    selection_start: Option<Point<Pixels>>,
    selection_current: Option<Point<Pixels>>,
    drag_nodes: Option<Vec<(usize, WorldPoint)>>,
    temporary_edge_preview: Option<(WorldPoint, WorldPoint)>,
    focus: FocusHandle,
    connection: ConnectionController,
    next_edge_id: u64,
    default_edge_marker_start: Option<EdgeMarker>,
    default_edge_marker_end: Option<EdgeMarker>,
    events: Vec<GraphEvent>,
    announcement: String,
    resize_node: Option<(usize, crate::NodeResizeControl)>,
    only_render_visible_elements: bool,
    selection_mode: SelectionMode,
    snap_grid: Option<crate::WorldSize>,
    auto_pan: bool,
    auto_pan_speed: f32,
    auto_pan_margin: f32,
    auto_pan_edge_since: Option<((i8, i8), std::time::Instant)>,
    edge_geometry_cache: crate::GeometryCache<Vec<WorldPoint>>,
    /// Closed-loop edge level of detail, and the clock it is driven from.
    governor: crate::DetailGovernor,
    last_frame: Option<std::time::Instant>,
    /// Duration of the previous frame and of the layout work inside it. The
    /// difference is the fixed cost a frame owes to everything that is not
    /// simulation, which is what the layout's slice of the deadline is
    /// computed against.
    last_frame_ms: f32,
    last_layout_ms: f32,
    scene: SceneCache,
    /// Dense cull output, retained across frames so a frame allocates nothing
    /// to answer "what is on screen". `visible_flags` is indexed by node index
    /// and lets an edge test both endpoints with two array reads.
    visible_indices: Rc<Vec<u32>>,
    visible_flags: Rc<Vec<bool>>,
    /// Node indices that have a registered element content renderer. Derived
    /// from node specs, so it is rebuilt only when the specs change.
    content_present: Rc<Vec<bool>>,
    /// Node indices that actually received an element this frame. The painter
    /// reads it to substitute a drawn body for every node the level-of-detail
    /// rule or the frame budget turned away.
    content_drawn: Rc<Vec<bool>>,
    pointer: Option<PointerController>,
    synced_membership_revision: u64,
    style_revision: u64,
    data_api: GraphDataApi,
    data_api_sync_revision: (u64, u64, u64, u64, u64),
    node_content_cache: HashMap<NodeId, Entity<CachedNodeContent>>,
    node_content_layer: Entity<NodeContentLayer>,
    node_content_layer_revision: Option<NodeContentLayerRevision>,
    /// Last laid-out graph bounds in window coordinates. The authoritative
    /// viewport remains graph-local; these bounds bridge GPUI pointer/paint
    /// coordinates at the component boundary.
    canvas_bounds: Option<Bounds<Pixels>>,
}

impl Graph {
    /// Screen pixels panned per frame when a gesture sits at the pane edge.
    pub const DEFAULT_AUTO_PAN_SPEED: f32 = 0.5;
    /// Width of the pane-edge band that triggers auto-pan, in screen pixels.
    pub const DEFAULT_AUTO_PAN_MARGIN: f32 = 28.0;
    /// Time a dragged node must remain against a pane edge before panning.
    pub const DEFAULT_AUTO_PAN_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

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
        let mut connection = ConnectionController::default();
        connection.set_validator(builder.connection_validator);
        Ok(Self {
            model,
            layout_edges,
            viewport: builder.viewport,
            renderer: builder.renderer,
            layout: builder.layout,
            layout_options: builder.layout_options,
            layout_positions: Vec::new(),
            layout_origins: Vec::new(),
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
            pointer_over_reconnect: false,
            smooth_zoom: None,
            gestures: GestureRouter::default(),
            selection_start: None,
            selection_current: None,
            drag_nodes: None,
            temporary_edge_preview: None,
            focus: cx.focus_handle().tab_stop(true),
            connection,
            next_edge_id,
            default_edge_marker_start: builder.default_edge_marker_start,
            default_edge_marker_end: builder.default_edge_marker_end,
            events: Vec::new(),
            announcement: String::new(),
            resize_node: None,
            only_render_visible_elements: builder.only_render_visible_elements,
            selection_mode: builder.selection_mode,
            snap_grid: builder.snap_grid,
            auto_pan: builder.auto_pan,
            auto_pan_speed: crate::style::finite_non_negative_or(
                builder.auto_pan_speed,
                Self::DEFAULT_AUTO_PAN_SPEED,
            ),
            auto_pan_margin: crate::style::finite_non_negative_or(
                builder.auto_pan_margin,
                Self::DEFAULT_AUTO_PAN_MARGIN,
            ),
            auto_pan_edge_since: None,
            edge_geometry_cache: crate::GeometryCache::default(),
            governor: crate::DetailGovernor::default(),
            last_frame: None,
            last_frame_ms: 0.0,
            last_layout_ms: 0.0,
            scene: SceneCache::default(),
            visible_indices: Rc::default(),
            visible_flags: Rc::default(),
            content_present: Rc::default(),
            content_drawn: Rc::default(),
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
            canvas_bounds: None,
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

    /// Replaces one directed edge with two edges routed through `node`.
    ///
    /// The source half retains the original edge ID and appearance. The target
    /// half receives `new_edge_id` and otherwise copies the original edge's
    /// configuration. The operation is validated and committed atomically.
    pub fn split_edge_at_node(
        &mut self,
        edge_id: crate::EdgeId,
        node: NodeId,
        new_edge_id: impl Into<u64>,
    ) -> Result<bool, GraphDataError> {
        let Some(edge) = self.model.edges.iter().find(|edge| edge.id == edge_id) else {
            return Ok(false);
        };
        if !self
            .model
            .nodes
            .iter()
            .any(|candidate| candidate.id == node)
        {
            return Ok(false);
        }

        let mut source_half = edge.clone();
        source_half.target = node;
        source_half.target_handle = None;
        let mut target_half = edge.clone();
        target_half.id = crate::EdgeId(new_edge_id.into());
        target_half.source = node;
        target_half.source_handle = None;
        let changes = [
            EdgeChange::Replace {
                id: edge_id,
                item: source_half,
            },
            EdgeChange::Add {
                index: None,
                item: target_half,
            },
        ];
        self.model.emit_graph_changes([], changes)?;
        Ok(true)
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

    /// Finds visible edges whose rendered path intersects a world-space area.
    ///
    /// Curved and custom routes use the same sampled geometry as painting.
    /// Straight routes use their endpoint segment. Touching the area boundary
    /// counts as an intersection.
    pub fn intersecting_edges(&self, area: WorldBounds) -> HashSet<crate::EdgeId> {
        self.model
            .edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| {
                let source = self.model.node(edge.source)?;
                let target = self.model.node(edge.target)?;
                if source.hidden || target.hidden {
                    return None;
                }
                let intersects = self
                    .scene
                    .edge_geometries
                    .get(index)
                    .and_then(Option::as_deref)
                    .map_or_else(
                        || {
                            segment_intersects_bounds(
                                self.node_center(source),
                                self.node_center(target),
                                area,
                            )
                        },
                        |path| {
                            path.windows(2)
                                .any(|pair| segment_intersects_bounds(pair[0], pair[1], area))
                        },
                    );
                intersects.then_some(edge.id)
            })
            .collect()
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
}

mod interaction;
mod layout_runtime;
mod render;
mod scene;
mod viewport;

use scene::{
    connection_handle_position, resize_handle_position, select_content_lod, selected_node_bounds,
    world_bounds, RESIZE_DIRECTIONS,
};

#[cfg(test)]
mod tests;

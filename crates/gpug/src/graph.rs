use std::rc::Rc;

use gpui::{canvas, div, *};

use crate::coordinates::{Viewport, WorldBounds, WorldPoint};
use crate::data::{GraphData, GraphDataError, LayoutEdge};
use crate::edge::Edge;
use crate::layout::{
    apply_fit, step_with_budget, AnimatedBatchLayout, BatchLayout, ForceAtlas2, Layout, LayoutFit,
    LayoutOptions, LayoutStatus,
};
use crate::node::{Node, NodeId};
use crate::renderer::GraphRenderer;
use crate::style::GraphStyle;

pub struct GraphBuilder {
    data: GraphData,
    layout: Box<dyn Layout>,
    layout_options: LayoutOptions,
    renderer: GraphRenderer,
    viewport: Viewport,
    interactive_layout: bool,
    fit_on_load: bool,
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

    pub fn build(self, _cx: &mut App) -> Result<Graph, GraphDataError> {
        Graph::from_builder(self)
    }
}

/// GPUG's developer-facing graph view and orchestration entity.
pub struct Graph {
    data: GraphData,
    layout_edges: Rc<[LayoutEdge]>,
    viewport: Viewport,
    renderer: GraphRenderer,
    layout: Box<dyn Layout>,
    layout_options: LayoutOptions,
    layout_initialized: bool,
    playing: bool,
    sim_tick: u64,
    fit_on_load_pending: bool,
    pan_drag_position: Option<Point<Pixels>>,
    pointer_over_graph_item: bool,
}

impl Graph {
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

    fn from_builder(builder: GraphBuilder) -> Result<Self, GraphDataError> {
        let layout_edges: Rc<[LayoutEdge]> = builder.data.compile_edges()?.into();
        Ok(Self {
            data: builder.data,
            layout_edges,
            viewport: builder.viewport,
            renderer: builder.renderer,
            layout: builder.layout,
            layout_options: builder.layout_options,
            layout_initialized: false,
            playing: builder.interactive_layout,
            sim_tick: 0,
            fit_on_load_pending: builder.fit_on_load,
            pan_drag_position: None,
            pointer_over_graph_item: false,
        })
    }

    pub fn data(&self) -> &GraphData {
        &self.data
    }

    pub fn set_data(&mut self, data: GraphData) -> Result<(), GraphDataError> {
        let compiled: Rc<[LayoutEdge]> = data.compile_edges()?.into();
        self.data = data;
        self.layout_edges = compiled;
        self.layout_initialized = false;
        self.sim_tick = 0;
        Ok(())
    }

    pub fn add_node(&mut self, node: Node) -> Result<(), GraphDataError> {
        let mut data = self.data.clone();
        data.nodes.push(node);
        self.set_data(data)
    }

    pub fn add_edge(&mut self, edge: Edge) -> Result<(), GraphDataError> {
        let mut data = self.data.clone();
        data.edges.push(edge);
        self.set_data(data)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.data.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.data.edges
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.data.node(id)
    }

    pub fn set_node_position(&mut self, id: NodeId, position: WorldPoint) -> bool {
        let Some(node) = self.data.node_mut(id) else {
            return false;
        };
        node.position = position;
        true
    }

    pub fn selected_nodes(&self) -> impl Iterator<Item = &Node> {
        self.data.nodes.iter().filter(|node| node.selected)
    }

    pub fn clear_selection(&mut self) {
        for node in &mut self.data.nodes {
            node.selected = false;
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
    }

    pub fn set_pan(&mut self, pan: Point<Pixels>) {
        self.viewport.set_pan(pan);
    }

    pub fn pan_by(&mut self, delta: Point<Pixels>) {
        let pan = self.viewport.pan();
        self.viewport
            .set_pan(point(pan.x + delta.x, pan.y + delta.y));
    }

    fn node_at_screen_position(&self, position: Point<Pixels>) -> Option<usize> {
        let hit_radius = px(self.renderer.style().hit_radius_pixels);
        self.data.nodes.iter().position(|node| {
            let center = self.viewport.world_to_screen(node.position);
            (center.x - position.x).abs() <= hit_radius
                && (center.y - position.y).abs() <= hit_radius
        })
    }

    fn edge_at_screen_position(&self, position: Point<Pixels>) -> bool {
        let point_x = position.x / px(1.0);
        let point_y = position.y / px(1.0);
        let tolerance_squared = self.renderer.style().hit_radius_pixels.powi(2);

        self.layout_edges.iter().any(|edge| {
            let start = self
                .viewport
                .world_to_screen(self.data.nodes[edge.source].position);
            let end = self
                .viewport
                .world_to_screen(self.data.nodes[edge.target].position);
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
            (point_x - nearest_x).powi(2) + (point_y - nearest_y).powi(2) <= tolerance_squared
        })
    }

    fn graph_item_at_screen_position(&self, position: Point<Pixels>) -> bool {
        self.node_at_screen_position(position).is_some() || self.edge_at_screen_position(position)
    }

    pub fn style(&self) -> &GraphStyle {
        self.renderer.style()
    }

    pub fn set_style(&mut self, style: GraphStyle) {
        self.renderer.set_style(style);
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
        self.viewport.set_zoom(zoom);
    }

    pub fn center_on(&mut self, id: NodeId, screen_center: Point<Pixels>) -> bool {
        let Some(position) = self.node(id).map(|node| node.position) else {
            return false;
        };
        self.viewport.set_pan(point(
            screen_center.x - px(position.x * self.viewport.zoom()),
            screen_center.y - px(position.y * self.viewport.zoom()),
        ));
        true
    }

    pub fn fit_to_view(&mut self, screen_size: Size<Pixels>, padding: Pixels) {
        let Some(bounds) = world_bounds(&self.data.nodes) else {
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

    pub fn step_layout(&mut self) -> LayoutStatus {
        if !self.layout_initialized {
            let positions: Vec<_> = self.data.nodes.iter().map(|node| node.position).collect();
            self.layout.initialize(&positions, &self.layout_edges);
            self.layout_initialized = true;
        }
        let mut positions: Vec<_> = self.data.nodes.iter().map(|node| node.position).collect();
        let status = if self.layout.use_frame_budget() {
            step_with_budget(
                self.layout.as_mut(),
                &mut positions,
                &self.layout_edges,
                self.layout_options.frame_budget,
            )
        } else {
            self.layout.step(&mut positions, &self.layout_edges)
        };
        if status == LayoutStatus::Converged {
            apply_fit(&mut positions, self.layout_options.fit);
        }
        for (node, position) in self.data.nodes.iter_mut().zip(positions) {
            node.position = position;
        }
        self.sim_tick = self.sim_tick.wrapping_add(1);
        if status == LayoutStatus::Converged {
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
            if last_status == LayoutStatus::Converged {
                break;
            }
        }
        self.layout_options.frame_budget = original_budget;
        last_status
    }
}

fn world_bounds(nodes: &[Node]) -> Option<WorldBounds> {
    let first = nodes.first()?.position;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.x, first.x, first.y, first.y);
    for node in &nodes[1..] {
        min_x = min_x.min(node.position.x);
        max_x = max_x.max(node.position.x);
        min_y = min_y.min(node.position.y);
        max_y = max_y.max(node.position.y);
    }
    Some(WorldBounds::new(
        WorldPoint::new(min_x, min_y),
        crate::WorldSize::new(max_x - min_x, max_y - min_y),
    ))
}

impl Render for Graph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.fit_on_load_pending {
            self.fit_to_view(window.viewport_size(), px(40.0));
            self.fit_on_load_pending = false;
        }
        let viewport = self.viewport;
        let renderer = self.renderer.clone();
        let style = renderer.style().clone();
        let positions: Rc<[WorldPoint]> = self
            .data
            .nodes
            .iter()
            .map(|node| node.position)
            .collect::<Vec<_>>()
            .into();
        let edges = self.layout_edges.clone();
        let edge_stride = renderer.interactive_edge_stride(edges.len(), self.playing);
        let selected: Rc<[usize]> = self
            .data
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.selected.then_some(index))
            .collect::<Vec<_>>()
            .into();

        let graph_canvas = canvas(
            |_bounds, _window, _cx| (),
            move |bounds, _, window, _cx| {
                let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                let mut edge_path = Path::new(point(px(0.0), px(0.0)));
                for edge in edges.iter().step_by(edge_stride) {
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

                let radius = px(renderer.node_radius_pixels(viewport.zoom()));
                let mut node_path = Path::new(point(px(0.0), px(0.0)));
                for position in positions.iter() {
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

                for &index in selected.iter() {
                    let center = viewport.world_to_screen(positions[index]);
                    window.paint_quad(outline(
                        Bounds::centered_at(center, size(px(18.0), px(18.0))),
                        rgb(style.selection_color),
                        BorderStyle::default(),
                    ));
                }
            },
        )
        .absolute()
        .size_full();

        let graph_handle = cx.entity();
        let simulation = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _, window, cx| {
                let playing = cx.read_entity(&graph_handle, |graph: &Graph, _| graph.playing);
                if playing {
                    window.request_animation_frame();
                    cx.update_entity(&graph_handle, |graph, cx| {
                        graph.step_layout();
                        cx.notify();
                    });
                }
            },
        )
        .absolute()
        .size_full();

        let playing = self.playing;
        let edge_count = self.data.edges.len();
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
                self.data.nodes.len(),
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
            .child(format!("layout frame: {}", self.sim_tick));

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
        } else if self.pointer_over_graph_item {
            CursorStyle::Arrow
        } else {
            CursorStyle::OpenHand
        };

        div()
            .size_full()
            .bg(rgb(self.renderer.style().background))
            .cursor(canvas_cursor)
            .child(simulation)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseDownEvent, _, cx| {
                    let hit = graph.node_at_screen_position(event.position);
                    if let Some(index) = hit {
                        if !event.modifiers.shift {
                            graph.clear_selection();
                        }
                        graph.data.nodes[index].selected = true;
                    } else if !graph.edge_at_screen_position(event.position) {
                        graph.pan_drag_position = Some(event.position);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|graph, event: &MouseMoveEvent, _, cx| {
                if let Some(previous) = graph.pan_drag_position {
                    graph.pan_by(point(
                        event.position.x - previous.x,
                        event.position.y - previous.y,
                    ));
                    graph.pan_drag_position = Some(event.position);
                    graph.pointer_over_graph_item = false;
                } else {
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseUpEvent, _, cx| {
                    graph.pan_drag_position = None;
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|graph, event: &ScrollWheelEvent, _, cx| {
                let dy = event.delta.pixel_delta(px(16.0)).y;
                if dy != px(0.0) {
                    let steps = ((dy / px(16.0)).abs()).max(0.01);
                    let factor = 1.1_f32.powf(steps);
                    let zoom = if dy > px(0.0) {
                        graph.viewport.zoom() * factor
                    } else {
                        graph.viewport.zoom() / factor
                    };
                    graph.viewport.zoom_about(event.position, zoom);
                    cx.notify();
                }
            }))
            .child(graph_canvas)
            .child(controls)
            .child(play_button)
    }
}

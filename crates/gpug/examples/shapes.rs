use std::borrow::Cow;

use gpug::{
    Edge, Graph, GraphData, GraphDataApi, GraphRenderer, Node, NodeAppearance, NodeId, NodeShape,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, svg, App, AppContext, Application, AssetSource,
    Bounds, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Pixels, Point, Render, SharedString, Window, WindowOptions,
};

const SIDEBAR_WIDTH: f32 = 184.0;
const SIDEBAR_HEIGHT: f32 = 158.0;
const DEFAULT_COLOR: u32 = 0x4a90e2;
const NODE_SIZE: WorldSize = WorldSize::new(12.0, 8.0);
const COLORS: [u32; 7] = [
    0xe04329, // red
    0xf39c35, // orange
    0xf4c542, // yellow
    0x3f8f5f, // green
    0x4a90e2, // blue
    0x4055b5, // indigo
    0x7c3aed, // violet
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowShape {
    Circle,
    RoundRectangle,
    Rectangle,
    Diamond,
    Hexagon,
    Parallelogram,
    Cylinder,
    Triangle,
    ArrowRectangle,
    Plus,
}

impl FlowShape {
    const ALL: [Self; 10] = [
        Self::Circle,
        Self::RoundRectangle,
        Self::Rectangle,
        Self::Diamond,
        Self::Hexagon,
        Self::Parallelogram,
        Self::Cylinder,
        Self::Triangle,
        Self::ArrowRectangle,
        Self::Plus,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::RoundRectangle => "round-rectangle",
            Self::Rectangle => "rectangle",
            Self::Diamond => "diamond",
            Self::Hexagon => "hexagon",
            Self::Parallelogram => "parallelogram",
            Self::Cylinder => "cylinder",
            Self::Triangle => "triangle",
            Self::ArrowRectangle => "arrow-rectangle",
            Self::Plus => "plus",
        }
    }

    fn asset(self) -> &'static str {
        match self {
            Self::Circle => "shape-circle.svg",
            Self::RoundRectangle => "shape-round-rectangle.svg",
            Self::Rectangle => "shape-rectangle.svg",
            Self::Diamond => "shape-diamond.svg",
            Self::Hexagon => "shape-hexagon.svg",
            Self::Parallelogram => "shape-parallelogram.svg",
            Self::Cylinder => "shape-cylinder.svg",
            Self::Triangle => "shape-triangle.svg",
            Self::ArrowRectangle => "shape-arrow-rectangle.svg",
            Self::Plus => "shape-plus.svg",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.key() == key)
    }

    fn size(self) -> WorldSize {
        match self {
            Self::Circle => WorldSize::new(10.0, 10.0),
            Self::Diamond => WorldSize::new(12.0, 12.0),
            Self::Triangle | Self::Plus => WorldSize::new(11.0, 11.0),
            Self::Cylinder => WorldSize::new(15.0, 11.0),
            _ => NODE_SIZE,
        }
    }

    fn contains(self, x: f32, y: f32) -> bool {
        if self == Self::Circle {
            return x * x + y * y <= 1.0;
        }
        let vertices = self.outline_points();
        let mut inside = false;
        let mut previous = vertices[vertices.len() - 1];
        for &current in &vertices {
            let crosses = (current.1 > y) != (previous.1 > y)
                && x < (previous.0 - current.0) * (y - current.1) / (previous.1 - current.1)
                    + current.0;
            if crosses {
                inside = !inside;
            }
            previous = current;
        }
        inside
    }

    fn outline_points(self) -> Vec<(f32, f32)> {
        match self {
            Self::Circle => (0..40)
                .map(|index| {
                    let angle = std::f32::consts::TAU * index as f32 / 40.0;
                    (angle.cos(), angle.sin())
                })
                .collect(),
            Self::RoundRectangle => rounded_rectangle_points(1.0, 1.0, 0.283, 0.425),
            Self::Rectangle => vec![(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)],
            Self::Diamond => vec![(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)],
            Self::Hexagon => vec![
                (-0.6, -1.0),
                (0.6, -1.0),
                (1.0, 0.0),
                (0.6, 1.0),
                (-0.6, 1.0),
                (-1.0, 0.0),
            ],
            Self::Parallelogram => vec![(-0.6, -1.0), (1.0, -1.0), (0.6, 1.0), (-1.0, 1.0)],
            Self::Cylinder => {
                let mut points = Vec::with_capacity(34);
                for step in 0..=16 {
                    let angle = std::f32::consts::PI + std::f32::consts::PI * step as f32 / 16.0;
                    points.push((angle.cos(), -0.7 + angle.sin() * 0.3));
                }
                for step in 0..=16 {
                    let angle = std::f32::consts::PI * step as f32 / 16.0;
                    points.push((angle.cos(), 0.7 + angle.sin() * 0.3));
                }
                points
            }
            Self::Triangle => vec![(0.0, -1.0), (1.0, 1.0), (-1.0, 1.0)],
            Self::ArrowRectangle => vec![
                (-1.0, -1.0),
                (0.6, -1.0),
                (1.0, 0.0),
                (0.6, 1.0),
                (-1.0, 1.0),
            ],
            Self::Plus => vec![
                (-0.25, -1.0),
                (0.25, -1.0),
                (0.25, -0.375),
                (1.0, -0.375),
                (1.0, 0.375),
                (0.25, 0.375),
                (0.25, 1.0),
                (-0.25, 1.0),
                (-0.25, 0.375),
                (-1.0, 0.375),
                (-1.0, -0.375),
                (-0.25, -0.375),
            ],
        }
    }
}

fn rounded_rectangle_points(hw: f32, hh: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(32);
    for (cx, cy, start) in [
        (hw - rx, -hh + ry, -std::f32::consts::FRAC_PI_2),
        (hw - rx, hh - ry, 0.0),
        (-hw + rx, hh - ry, std::f32::consts::FRAC_PI_2),
        (-hw + rx, -hh + ry, std::f32::consts::PI),
    ] {
        for step in 0..8 {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / 7.0;
            points.push((cx + angle.cos() * rx, cy + angle.sin() * ry));
        }
    }
    points
}

const CIRCLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><ellipse cx="60" cy="40" rx="60" ry="40" fill="#fff"/></svg>"##;
const ROUND_RECTANGLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><rect width="120" height="80" rx="17" fill="#fff"/></svg>"##;
const RECTANGLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><rect width="120" height="80" fill="#fff"/></svg>"##;
const DIAMOND: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M60 0L120 40 60 80 0 40Z" fill="#fff"/></svg>"##;
const HEXAGON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M24 0H96L120 40 96 80H24L0 40Z" fill="#fff"/></svg>"##;
const PARALLELOGRAM: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M24 0H120L96 80H0Z" fill="#fff"/></svg>"##;
const CYLINDER: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M0 12C0 0 120 0 120 12V68C120 80 0 80 0 68Z" fill="#fff"/><path d="M0 12C0 24 120 24 120 12" fill="none" stroke="#fff" stroke-width="4"/></svg>"##;
const TRIANGLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M60 0L120 80H0Z" fill="#fff"/></svg>"##;
const ARROW_RECTANGLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M0 0H96L120 40 96 80H0Z" fill="#fff"/></svg>"##;
const PLUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 80" preserveAspectRatio="none"><path d="M45 0H75V25H120V55H75V80H45V55H0V25H45Z" fill="#fff"/></svg>"##;

struct ShapeAssets;

impl AssetSource for ShapeAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let bytes = match path {
            "shape-circle.svg" => CIRCLE,
            "shape-round-rectangle.svg" => ROUND_RECTANGLE,
            "shape-rectangle.svg" => RECTANGLE,
            "shape-diamond.svg" => DIAMOND,
            "shape-hexagon.svg" => HEXAGON,
            "shape-parallelogram.svg" => PARALLELOGRAM,
            "shape-cylinder.svg" => CYLINDER,
            "shape-triangle.svg" => TRIANGLE,
            "shape-arrow-rectangle.svg" => ARROW_RECTANGLE,
            "shape-plus.svg" => PLUS,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, prefix: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FlowShape::ALL
            .into_iter()
            .map(FlowShape::asset)
            .filter(|path| path.starts_with(prefix))
            .map(SharedString::from)
            .collect())
    }
}

fn parse_color(node: &Node) -> u32 {
    node.metadata
        .get("color")
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .unwrap_or(DEFAULT_COLOR)
}

fn border_color(color: u32) -> u32 {
    let channel = |shift| (((color >> shift) & 0xff_u32) as f32 * 0.78) as u32;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn shape_path(
    shape: FlowShape,
    bounds: Bounds<Pixels>,
    inset: Pixels,
) -> Option<gpui::Path<Pixels>> {
    let center = bounds.center();
    let half_width = (bounds.size.width * 0.5 - inset).max(px(0.0));
    let half_height = (bounds.size.height * 0.5 - inset).max(px(0.0));
    let points = shape
        .outline_points()
        .into_iter()
        .map(|(x, y)| point(center.x + half_width * x, center.y + half_height * y))
        .collect::<Vec<_>>();
    let mut path = PathBuilder::fill();
    path.add_polygon(&points, true);
    path.build().ok()
}

fn shape_canvas(
    shape: FlowShape,
    color: u32,
    border: u32,
    border_width: Pixels,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if let Some(path) = shape_path(shape, bounds, px(0.0)) {
                window.paint_path(path, rgb(border));
            }
            if let Some(path) = shape_path(shape, bounds, border_width) {
                window.paint_path(path, rgb(color));
            }

            if shape == FlowShape::Cylinder {
                let inset = border_width;
                let left = bounds.left() + inset;
                let right = bounds.right() - inset;
                let top = bounds.top() + inset;
                let seam_y = top + (bounds.size.height - inset * 2.0) * 0.15;
                let dip_y = top + (bounds.size.height - inset * 2.0) * 0.30;
                let mut seam = PathBuilder::stroke(border_width.max(px(1.0)));
                seam.move_to(point(left, seam_y));
                seam.cubic_bezier_to(
                    point(right, seam_y),
                    point(left, dip_y),
                    point(right, dip_y),
                );
                if let Ok(path) = seam.build() {
                    window.paint_path(path, rgb(border));
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}

fn flow_node(node: &Node, zoom: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        // GPUG remains shape-agnostic: its default left/right connection
        // handles sit on the exterior of this node's measured bounding box.
        radius_pixels: node.size.width * zoom * 0.5,
        shape: NodeShape::None,
    }
}

fn make_node(id: u64, shape: FlowShape, position: WorldPoint, color: u32) -> Node {
    let mut node = Node::new(id, position)
        .with_size(shape.size())
        .with_type(shape.key());
    // GPUG owns selection, dragging, resizing, and optional edge auto-pan.
    node.selectable = true;
    node.draggable = true;
    node.metadata.insert("color".into(), format!("{color:06x}"));
    node.metadata.insert("caption".into(), shape.key().into());
    node
}

struct ShapesExample {
    graph: Entity<Graph>,
    data_api: GraphDataApi,
    dragging: Option<FlowShape>,
    selected: Option<NodeId>,
    pointer: Point<Pixels>,
}

impl ShapesExample {
    fn shape_button(&self, shape: FlowShape, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(shape.key())
            .size(px(42.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .hover(|style| style.bg(rgb(0xe8edf3)))
            .cursor_grab()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    view.dragging = Some(shape);
                    view.pointer = event.position;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                svg()
                    .path(shape.asset())
                    .size(px(32.0))
                    .text_color(rgb(0x89919a)),
            )
    }

    fn selected_node(&self, cx: &App) -> Option<NodeId> {
        cx.read_entity(&self.graph, |graph, _| {
            graph.selected_nodes().next().map(|node| node.id)
        })
    }

    fn set_selected_color(&mut self, color: u32, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_node(cx) {
            self.data_api
                .update_node_data(id, [("color".into(), format!("{color:06x}"))]);
            cx.refresh_windows();
        }
    }

    fn drop_shape(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(shape) = self.dragging.take() else {
            return;
        };
        if event.position.x < px(SIDEBAR_WIDTH + 16.0)
            && event.position.y < px(SIDEBAR_HEIGHT + 16.0)
        {
            cx.notify();
            return;
        }
        let mut added = None;
        cx.update_entity(&self.graph, |graph, graph_cx| {
            let id = graph
                .nodes()
                .iter()
                .map(|node| node.id.0)
                .max()
                .unwrap_or(0)
                + 1;
            let position = graph.screen_to_world(event.position);
            if graph
                .add_node(make_node(id, shape, position, DEFAULT_COLOR))
                .is_ok()
            {
                added = Some(NodeId(id));
                graph.clear_selection();
                graph.select_node(NodeId(id));
                graph_cx.notify();
            }
        });
        self.selected = added;
        cx.notify();
    }

    fn begin_node_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.dragging.is_some() {
            return;
        }
        let (on_resize_control, bounding_node, hit) = cx.read_entity(&self.graph, |graph, _| {
            let on_resize_control = self.selected.is_some_and(|id| {
                let Some(bounds) = graph.node_bounds(id) else {
                    return false;
                };
                let top_left = graph.world_to_screen(bounds.origin);
                let bottom_right = graph.world_to_screen(WorldPoint::new(
                    bounds.origin.x + bounds.size.width,
                    bounds.origin.y + bounds.size.height,
                ));
                let center = point(
                    (top_left.x + bottom_right.x) * 0.5,
                    (top_left.y + bottom_right.y) * 0.5,
                );
                [
                    top_left,
                    point(center.x, top_left.y),
                    point(bottom_right.x, top_left.y),
                    point(bottom_right.x, center.y),
                    bottom_right,
                    point(center.x, bottom_right.y),
                    point(top_left.x, bottom_right.y),
                    point(top_left.x, center.y),
                ]
                .into_iter()
                .any(|handle| {
                    (handle.x - event.position.x).abs() <= px(10.0)
                        && (handle.y - event.position.y).abs() <= px(10.0)
                })
            });
            let mut bounding_node = None;
            let hit = graph.nodes().iter().rev().find_map(|node| {
                let shape = FlowShape::from_key(&node.node_type)?;
                let bounds = graph.node_bounds(node.id)?;
                let top_left = graph.world_to_screen(bounds.origin);
                let bottom_right = graph.world_to_screen(WorldPoint::new(
                    bounds.origin.x + bounds.size.width,
                    bounds.origin.y + bounds.size.height,
                ));
                let half_width = (bottom_right.x - top_left.x) * 0.5;
                let half_height = (bottom_right.y - top_left.y) * 0.5;
                let center = point(top_left.x + half_width, top_left.y + half_height);
                if event.position.x >= top_left.x
                    && event.position.x <= bottom_right.x
                    && event.position.y >= top_left.y
                    && event.position.y <= bottom_right.y
                    && bounding_node.is_none()
                {
                    bounding_node = Some(node.id);
                }
                let x = (event.position.x - center.x) / half_width;
                let y = (event.position.y - center.y) / half_height;
                shape.contains(x, y).then(|| {
                    let pointer = graph.screen_to_world(event.position);
                    let center = WorldPoint::new(
                        bounds.origin.x + bounds.size.width * 0.5,
                        bounds.origin.y + bounds.size.height * 0.5,
                    );
                    (
                        node.id,
                        WorldPoint::new(pointer.x - center.x, pointer.y - center.y),
                    )
                })
            });
            (on_resize_control, bounding_node, hit)
        });
        if on_resize_control {
            return;
        }
        if let Some((id, _)) = hit {
            cx.update_entity(&self.graph, |graph, graph_cx| {
                graph.clear_selection();
                graph.select_node(id);
                graph_cx.notify();
            });
            self.selected = Some(id);
            cx.notify();
        } else if bounding_node.is_some() {
            cx.update_entity(&self.graph, |graph, graph_cx| {
                graph.clear_selection();
                graph_cx.notify();
            });
            self.selected = None;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn toolbar_position(&self, cx: &App) -> Option<Point<Pixels>> {
        let id = self.selected_node(cx)?;
        cx.read_entity(&self.graph, |graph, _| {
            let bounds = graph.node_bounds(id)?;
            let top_left = graph.world_to_screen(bounds.origin);
            let bottom_right = graph.world_to_screen(WorldPoint::new(
                bounds.origin.x + bounds.size.width,
                bounds.origin.y + bounds.size.height,
            ));
            Some(point(
                ((top_left.x + bottom_right.x) * 0.5 - px(126.0)).max(px(208.0)),
                (top_left.y - px(52.0)).max(px(10.0)),
            ))
        })
    }
}

impl Render for ShapesExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = div()
            .absolute()
            .left(px(12.0))
            .top(px(12.0))
            .w(px(SIDEBAR_WIDTH))
            .p(px(12.0))
            .rounded(px(8.0))
            .bg(rgb(0xf8fafc))
            .border(px(1.0))
            .border_color(rgb(0xd8dde3))
            .shadow_sm()
            .child(
                div()
                    .mb(px(10.0))
                    .text_size(px(14.0))
                    .text_color(rgb(0x25292e))
                    .child("Drag shapes to the canvas"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.0))
                    .children(FlowShape::ALL.map(|shape| self.shape_button(shape, cx))),
            );

        let mut root = div()
            .id("shapes-example")
            .relative()
            .size_full()
            .overflow_hidden()
            .cursor(gpui::CursorStyle::Arrow)
            .child(self.graph.clone())
            .child(sidebar)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, cx| view.begin_node_drag(event, cx)),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                if view.dragging.is_some() {
                    view.pointer = event.position;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _, cx| {
                    if view.dragging.is_some() {
                        view.drop_shape(event, cx);
                    }
                }),
            );

        if let Some(toolbar) = self.toolbar_position(cx) {
            root = root.child(
                div()
                    .absolute()
                    .top(toolbar.y)
                    .left(toolbar.x)
                    .p(px(6.0))
                    .flex()
                    .gap(px(7.0))
                    .rounded_full()
                    .bg(rgb(0xffffff))
                    .border(px(1.0))
                    .border_color(rgb(0xd8dde3))
                    .shadow_md()
                    .children(COLORS.map(|color| {
                        div()
                            .id(("node-color", color))
                            .size(px(26.0))
                            .rounded_full()
                            .bg(rgb(color))
                            .border(px(2.0))
                            .border_color(rgb(0xffffff))
                            .shadow_sm()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                                    view.set_selected_color(color, cx);
                                    cx.stop_propagation();
                                }),
                            )
                    })),
            );
        }

        if let Some(shape) = self.dragging {
            root = root.child(
                div()
                    .absolute()
                    .left(self.pointer.x - px(25.0))
                    .top(self.pointer.y - px(18.0))
                    .w(px(50.0))
                    .h(px(36.0))
                    .opacity(0.72)
                    .child(
                        svg()
                            .path(shape.asset())
                            .size_full()
                            .text_color(rgb(DEFAULT_COLOR)),
                    ),
            );
        }
        root
    }
}

fn initial_data() -> GraphData {
    let specs = [
        (
            FlowShape::RoundRectangle,
            WorldPoint::new(-14.0, -15.0),
            0x4a90e2,
        ),
        (FlowShape::Diamond, WorldPoint::new(-14.0, -3.0), 0xf39c35),
        (FlowShape::Circle, WorldPoint::new(-32.0, -3.0), 0x3f8f5f),
        (FlowShape::Hexagon, WorldPoint::new(4.0, -3.0), 0xe05a3a),
        (FlowShape::Cylinder, WorldPoint::new(-14.0, 10.0), 0xf4c542),
        (
            FlowShape::ArrowRectangle,
            WorldPoint::new(-32.0, 10.0),
            0x7c3aed,
        ),
        (FlowShape::Rectangle, WorldPoint::new(22.0, 8.0), 0x3f8f5f),
        (
            FlowShape::Parallelogram,
            WorldPoint::new(4.0, 20.0),
            0x7c3aed,
        ),
        (FlowShape::Plus, WorldPoint::new(-32.0, 23.0), 0xe05a3a),
        (FlowShape::Triangle, WorldPoint::new(-14.0, 27.0), 0x4a90e2),
    ];
    let nodes = specs
        .into_iter()
        .enumerate()
        .map(|(index, (shape, position, color))| {
            make_node(index as u64 + 1, shape, position, color)
        })
        .collect();
    let edges = [
        (1_u64, 2_u64),
        (2, 3),
        (2, 4),
        (2, 5),
        (3, 6),
        (6, 5),
        (4, 7),
        (7, 8),
        (6, 9),
        (9, 10),
        (5, 10),
    ]
    .into_iter()
    .map(|(source, target)| {
        let mut edge = Edge::new(source, target);
        edge.edge_type = "smoothstep".into();
        edge
    })
    .collect();
    GraphData::new(nodes, edges)
}

fn main() {
    Application::new()
        .with_assets(ShapeAssets)
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    app_id: Some("GPUG — Flow-chart Shapes".into()),
                    ..Default::default()
                },
                |_, cx| {
                    let data_api = GraphDataApi::new();
                    let mut renderer = GraphRenderer::default();
                    for shape in FlowShape::ALL {
                        renderer.register_node_type(shape.key(), flow_node);
                        renderer.register_node_content(
                            shape.key(),
                            move |node: &Node, zoom: f32| {
                                let color = parse_color(node);
                                let inset = px((0.18 * zoom).max(1.5));
                                div()
                                    .relative()
                                    .size_full()
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(shape_canvas(shape, color, border_color(color), inset))
                                    .child(
                                        div()
                                            .text_size(px((1.25 * zoom).clamp(8.0, 13.0)))
                                            .text_color(rgb(0x263238))
                                            .child(
                                                node.metadata
                                                    .get("caption")
                                                    .cloned()
                                                    .unwrap_or_default(),
                                            ),
                                    )
                                    .into_any_element()
                            },
                        );
                    }
                    let graph = cx.new(|cx| {
                        Graph::builder()
                            .data(initial_data())
                            .data_api(data_api.clone())
                            .renderer(renderer)
                            .auto_pan(true)
                            .fit_on_load()
                            .show_resize_handles(true)
                            .build(cx)
                            .unwrap()
                    });
                    cx.new(|_| ShapesExample {
                        graph,
                        data_api,
                        dragging: None,
                        selected: None,
                        pointer: point(px(0.0), px(0.0)),
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_contains_the_expected_flow_chart_shapes() {
        assert!(FlowShape::ALL.contains(&FlowShape::Circle));
        assert!(FlowShape::ALL.contains(&FlowShape::Diamond));
        assert!(FlowShape::ALL.contains(&FlowShape::Hexagon));
    }

    #[test]
    fn initial_nodes_have_renderable_colors() {
        assert!(initial_data()
            .nodes
            .iter()
            .all(|node| parse_color(node) != 0));
    }

    #[test]
    fn initial_edges_use_smoothstep_routing() {
        assert!(initial_data()
            .edges
            .iter()
            .all(|edge| edge.edge_type == "smoothstep"));
    }

    #[test]
    fn shape_assets_stretch_to_the_resized_node_bounds() {
        for asset in [
            CIRCLE,
            ROUND_RECTANGLE,
            RECTANGLE,
            DIAMOND,
            HEXAGON,
            PARALLELOGRAM,
            CYLINDER,
            TRIANGLE,
            ARROW_RECTANGLE,
            PLUS,
        ] {
            assert!(std::str::from_utf8(asset)
                .unwrap()
                .contains("preserveAspectRatio=\"none\""));
        }
    }

    #[test]
    fn custom_hit_tests_reject_transparent_bounding_box_corners() {
        assert!(FlowShape::Diamond.contains(0.0, 0.0));
        assert!(!FlowShape::Diamond.contains(0.9, 0.9));
        assert!(FlowShape::Hexagon.contains(0.0, 0.0));
        assert!(!FlowShape::Hexagon.contains(0.95, 0.85));
        assert!(!FlowShape::Circle.contains(0.8, 0.8));
    }

    #[test]
    fn custom_geometry_reaches_every_edge_of_the_node_bounds() {
        for shape in FlowShape::ALL {
            let points = shape.outline_points();
            let min_x = points
                .iter()
                .map(|point| point.0)
                .fold(f32::INFINITY, f32::min);
            let max_x = points
                .iter()
                .map(|point| point.0)
                .fold(f32::NEG_INFINITY, f32::max);
            let min_y = points
                .iter()
                .map(|point| point.1)
                .fold(f32::INFINITY, f32::min);
            let max_y = points
                .iter()
                .map(|point| point.1)
                .fold(f32::NEG_INFINITY, f32::max);
            for extent in [min_x, min_y, -max_x, -max_y] {
                assert!((extent + 1.0).abs() < 0.001, "{shape:?} has inset geometry");
            }
        }
    }

    #[test]
    fn graph_nodes_use_framework_selection_and_dragging() {
        assert!(initial_data()
            .nodes
            .iter()
            .all(|node| node.selectable && node.draggable));
    }
}

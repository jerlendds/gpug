use gpui::*;
use gpui::{
    Application, Context, IntoElement, ParentElement, Render, Styled, Window, WindowOptions, canvas, div,
};

// Simple draggable node
pub struct GpugNode {
    id: u64,
    x: Pixels,
    y: Pixels,
    // Offset from the node's origin to the cursor at drag start
    drag_offset: Option<Point<Pixels>>,
}

impl Render for GpugNode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left(self.x)
            .top(self.y)
            .size(px(16.0))
            .rounded_full()
            .bg(rgb(0x00ffff))
            .cursor_move()
            // Make this element stateful so `.on_drag` is available
            .id(("node", self.id as usize))
            // Start a drag with this node's id as payload; lets listeners filter events
            .on_drag(self.id, |_id: &u64, _offset, _window, cx| cx.new(|_| DragPreview))
            // Update position while dragging only if this node is the dragged one
            .on_drag_move::<u64>(cx.listener(|this, event: &DragMoveEvent<u64>, _window, cx| {
                if *event.drag(cx) != this.id {
                    return;
                }
                // Record the initial cursor offset inside the node on first move
                if this.drag_offset.is_none() {
                    let offset = point(
                        event.event.position.x - event.bounds.left(),
                        event.event.position.y - event.bounds.top(),
                    );
                    this.drag_offset = Some(offset);
                }

                if let Some(offset) = this.drag_offset {
                    let new_origin = point(
                        event.event.position.x - offset.x,
                        event.event.position.y - offset.y,
                    );
                    this.x = new_origin.x;
                    this.y = new_origin.y;
                    // Notify to re-render with new position
                    cx.notify();
                }
            }))
            // Clear drag state on drop
            .on_drop(cx.listener(|this, dragged_id: &u64, _window, cx| {
                if *dragged_id == this.id {
                    this.drag_offset = None;
                    cx.notify();
                }
            }))
    }
}

// Minimal drag preview view to satisfy on_drag constructor
struct DragPreview;
impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Invisible 1x1 element as drag ghost
        div().size(px(1.0)).bg(rgb(0x000000)).opacity(0.0)
    }
}

// Root view that hosts nodes
struct Graph {
    nodes: Vec<Entity<GpugNode>>,
    edges: Vec<Entity<GpugEdge>>,
}

fn generate_1000_nodes() {
    // TODO
}

fn randomly_connect_nodes_with_edges() {
    // TODO
}


impl Graph {
    fn new(cx: &mut App) -> Self {

        let nodes: Vec<Entity<GpugNode>> = vec![cx.new(|_| GpugNode {
            id: 1,
            x: px(100.0),
            y: px(100.0),
            drag_offset: None,
        }),
        cx.new(|_| GpugNode {
            id: 2,
            x: px(120.0),
            y: px(190.0),
            drag_offset: None,
        }),
        cx.new(|_| GpugNode {
            id: 3,
            x: px(150.0),
            y: px(200.0),
            drag_offset: None,
        })];
        // Edge between node1 and node3
        let edge1 = cx.new(|_| GpugEdge::new(nodes[0].clone(), nodes[2].clone()));
        Self {
            nodes,
            edges: vec![edge1],
        }
    }
}

impl Render for Graph {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Build the canvas and add nodes without moving out of self.nodes
        let graph_canvas = div()
            .size_full()
            // Paint edges first so nodes render above lines
            .children(self.edges.iter().cloned())
            .children(self.nodes.iter().cloned());

        div().size_full().bg(rgb(0x2e7d32)).child(graph_canvas)
    }
}

// Edge connecting two nodes by drawing a straight path between their centers
struct GpugEdge {
    from: Entity<GpugNode>,
    to: Entity<GpugNode>,
}

impl GpugEdge {
    fn new(from: Entity<GpugNode>, to: Entity<GpugNode>) -> Self {
        Self { from, to }
    }
}

impl Render for GpugEdge {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read node positions (centers) at render time
        let (x1, y1) = cx.read_entity(&self.from, |n, _| (n.x + px(8.0), n.y + px(8.0)));
        let (x2, y2) = cx.read_entity(&self.to, |n, _| (n.x + px(8.0), n.y + px(8.0)));

        let p1 = point(x1, y1);
        let p2 = point(x2, y2);

        // Use gpui's canvas element to paint a thin rectangle between points
        canvas(
            move |_bounds, _window, _cx| (p1, p2),
            |_bounds, (p1, p2), window, _cx| {
                let dir = point(p2.x - p1.x, p2.y - p1.y);
                let len = dir.magnitude() as f32;
                if len <= 0.0001 {
                    return;
                }
                // Increase thickness: half_thickness = 3px => 6px total line thickness
                let half_thickness: f32 = 1.0;
                let normal = point(-dir.y, dir.x) * (half_thickness / len);

                let p1a = point(p1.x + normal.x, p1.y + normal.y);
                let p1b = point(p1.x - normal.x, p1.y - normal.y);
                let p2a = point(p2.x + normal.x, p2.y + normal.y);
                let p2b = point(p2.x - normal.x, p2.y - normal.y);

                let mut path = gpui::Path::new(p1a);
                // Use constant ST for solid fill; avoids tapered alpha in path shader
                let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                path.push_triangle((p1a, p1b, p2a), st);
                path.push_triangle((p2a, p1b, p2b), st);

                window.paint_path(path, gpui::red());
            },
        )
        .absolute()
        .size_full()
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|cx| Graph::new(cx)))
            .unwrap();
    });
}

use gpui::div;
use gpui::*;

// Simple draggable node
pub struct GpugNode {
    pub id: u64,
    pub x: Pixels,
    pub y: Pixels,
    // Offset from the node's origin to the cursor at drag start
    pub drag_offset: Option<Point<Pixels>>,
    pub zoom: f32,
    pub pan: Point<Pixels>,
    pub selected: bool,
}

impl Render for GpugNode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let node = div()
            .size(px(16.0 * self.zoom))
            .rounded_full()
            .bg(rgb(0x000000))
            .cursor_move()
            .id(("node", self.id as usize))
            // Start a drag with this node's id as payload; lets listeners filter events
            .on_drag(self.id, |_id: &u64, _offset, _window, cx| {
                cx.new(|_| DragPreview)
            })
            // Update position while dragging only if this node is the dragged one
            .on_drag_move::<u64>(
                cx.listener(|this, event: &DragMoveEvent<u64>, _window, cx| {
                    if *event.drag(cx) != this.id {
                        return;
                    }
                    // Record the initial cursor offset inside the node on first move
                    if this.drag_offset.is_none() {
                        let offset = point(
                            (event.event.position.x - event.bounds.left()) / this.zoom,
                            (event.event.position.y - event.bounds.top()) / this.zoom,
                        );
                        this.drag_offset = Some(offset);
                    }

                    if let Some(offset) = this.drag_offset {
                        let new_origin = point(
                            (event.event.position.x - this.pan.x) / this.zoom - offset.x,
                            (event.event.position.y - this.pan.y) / this.zoom - offset.y,
                        );
                        this.x = new_origin.x;
                        this.y = new_origin.y;
                        // position changes trigger re-render
                    }
                }),
            )
            .on_drop(cx.listener(|this, dragged_id: &u64, _window, _cx| {
                if *dragged_id == this.id {
                    this.drag_offset = None;
                }
            }));

        if self.selected {
            // Wrap the dot with a positioned container that provides
            // a fixed 10px gap between the border and the black node
            div()
                .absolute()
                .left(self.pan.x + self.x * self.zoom - px(10.0))
                .top(self.pan.y + self.y * self.zoom - px(10.0))
                .p(px(8.0))
                .border(px(4.0))
                .rounded_full()
                .border_color(rgb(0x1E90FF))
                .child(node)
        } else {
            // Unselected: position with a wrapper so both branches return the same type
            div()
                .absolute()
                .left(self.pan.x + self.x * self.zoom)
                .top(self.pan.y + self.y * self.zoom)
                .child(node)
        }
    }
}

// Minimal drag preview view to satisfy on_drag constructor
struct DragPreview;
impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Invisible 1x1 element as drag ghost
        div().size(px(1.0)).bg(rgb(0xffffff)).opacity(0.0)
    }
}

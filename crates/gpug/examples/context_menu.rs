use gpug::{ContextMenuTarget, Edge, Graph, GraphData, Node, NodeId, WorldPoint, WorldSize};
use gpui::{
    App, AppContext, Application, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point,
    Render, Window, WindowOptions, div, point, prelude::*, px, rgb,
};

struct OpenMenu {
    target: ContextMenuTarget,
    position: Point<Pixels>,
    graph_position: WorldPoint,
}

struct ContextMenuExample {
    graph: Entity<Graph>,
    menu: Option<OpenMenu>,
}

impl ContextMenuExample {
    fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
    }

    fn delete_target(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        cx.update_entity(&self.graph, |graph, graph_cx| {
            match menu.target {
                ContextMenuTarget::Node(id) => {
                    graph.select_node(id);
                }
                ContextMenuTarget::Edge(id) => {
                    graph.select_edge(id);
                }
                ContextMenuTarget::Selection { .. } => {}
                ContextMenuTarget::Pane => return,
            }
            graph.delete_selected();
            graph_cx.notify();
        });
        cx.notify();
    }

    fn duplicate_target(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        cx.update_entity(&self.graph, |graph, graph_cx| {
            if let ContextMenuTarget::Node(id) = menu.target {
                if let Some(mut node) = graph.node(id).cloned() {
                    node.id = NodeId(
                        graph
                            .nodes()
                            .iter()
                            .map(|node| node.id.0)
                            .max()
                            .unwrap_or(0)
                            + 1,
                    );
                    node.position = WorldPoint::new(node.position.x + 3.0, node.position.y + 3.0);
                    node.selected = false;
                    let _ = graph.add_node(node);
                    graph_cx.notify();
                }
            }
        });
        cx.notify();
    }

    fn add_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        cx.update_entity(&self.graph, |graph, graph_cx| {
            let id = NodeId(
                graph
                    .nodes()
                    .iter()
                    .map(|node| node.id.0)
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
            let node = Node::new(id, menu.graph_position).with_size(WorldSize::new(11.0, 7.0));
            let _ = graph.add_node(node);
            graph_cx.notify();
        });
        cx.notify();
    }
}

impl Render for ContextMenuExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let graph = self.graph.clone();
        let mut root = div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _: &MouseDownEvent, _, cx| view.close_menu(cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    let target = cx
                        .read_entity(&graph, |graph, _| graph.context_menu_target(event.position));
                    let graph_position =
                        cx.read_entity(&graph, |graph, _| graph.screen_to_world(event.position));
                    let viewport = window.viewport_size();
                    let x = event
                        .position
                        .x
                        .min(viewport.width - px(132.0))
                        .max(px(4.0));
                    let y = event
                        .position
                        .y
                        .min(viewport.height - px(118.0))
                        .max(px(4.0));
                    view.menu = Some(OpenMenu {
                        target,
                        position: point(x, y),
                        graph_position,
                    });
                    cx.stop_propagation();
                    cx.notify();
                }),
            );

        if let Some(menu) = &self.menu {
            let (title, actions): (String, &[&str]) = match &menu.target {
                ContextMenuTarget::Node(id) => {
                    (format!("node: {}", id.0), &["duplicate", "delete"])
                }
                ContextMenuTarget::Edge(id) => (format!("edge: {}", id.0), &["delete"]),
                ContextMenuTarget::Selection { nodes, edges } => (
                    format!("selection: {}", nodes.len() + edges.len()),
                    &["delete"],
                ),
                ContextMenuTarget::Pane => ("pane".into(), &["add node"]),
            };
            let mut overlay = div()
                .absolute()
                .left(menu.position.x)
                .top(menu.position.y)
                .w(px(124.0))
                .bg(rgb(0xf7f7f7))
                .border(px(1.0))
                .border_color(rgb(0x202124))
                .shadow_md()
                .cursor_default()
                .child(
                    div()
                        .px_2()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(0xd0d0d0))
                        .child(title),
                );
            for action in actions.iter().copied() {
                overlay = overlay.child(
                    div()
                        .px_2()
                        .py_2()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0xe5e7eb)))
                        .child(action)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                                match action {
                                    "duplicate" => view.duplicate_target(cx),
                                    "delete" => view.delete_target(cx),
                                    "add node" => view.add_node(cx),
                                    _ => view.close_menu(cx),
                                }
                                cx.stop_propagation();
                            }),
                        ),
                );
            }
            root = root.child(overlay);
        }
        root
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Context Menu".into()),
                ..Default::default()
            },
            |_, cx| {
                let nodes = vec![
                    Node::new(1_u64, WorldPoint::new(-12.0, 0.0))
                        .with_size(WorldSize::new(11.0, 7.0)),
                    Node::new(2_u64, WorldPoint::new(12.0, 0.0))
                        .with_size(WorldSize::new(11.0, 7.0)),
                ];
                let edges = vec![Edge::new(1_u64, 2_u64).with_id(1_u64)];
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, edges))
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                cx.new(|_| ContextMenuExample { graph, menu: None })
            },
        )
        .unwrap();
    });
}

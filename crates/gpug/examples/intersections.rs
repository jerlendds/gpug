use gpug::{
    Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeId, NodeShape, SelectionMode,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, px, rgb, App, AppContext, Application, Context, Entity, IntoElement,
    ParentElement, Render, Styled, Window, WindowOptions,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum IntersectionKind {
    Partial,
    FullyContained,
}

struct IntersectionOverlay {
    graph: Entity<Graph>,
    highlighted: Arc<Mutex<HashMap<NodeId, IntersectionKind>>>,
    displayed: Vec<(NodeId, IntersectionKind)>,
    dragging: Option<NodeId>,
    active_node: Option<NodeId>,
}

impl IntersectionOverlay {
    fn sample(&mut self, cx: &App) -> bool {
        let previous_active = self.active_node;
        let (dragging, active_node, ids) = cx.read_entity(&self.graph, |graph, _| {
            let dragging = graph.nodes().iter().find_map(|node| {
                graph
                    .editor()
                    .runtimes
                    .get(&node.id)?
                    .dragging
                    .then_some(node.id)
            });
            let active_node = dragging.or(previous_active);
            let highlighted = active_node.map_or_else(HashMap::new, |id| {
                let fully_contained = graph.get_intersecting_nodes(id, SelectionMode::Full);
                graph
                    .get_intersecting_nodes(id, SelectionMode::Partial)
                    .into_iter()
                    .map(|candidate| {
                        let kind = if fully_contained.contains(&candidate) {
                            IntersectionKind::FullyContained
                        } else {
                            IntersectionKind::Partial
                        };
                        (candidate, kind)
                    })
                    .collect()
            });
            (dragging, active_node, highlighted)
        });
        let mut displayed = ids
            .iter()
            .map(|(id, kind)| (*id, *kind))
            .collect::<Vec<_>>();
        displayed.sort_unstable();
        if self.dragging == dragging
            && self.active_node == active_node
            && self.displayed == displayed
        {
            return false;
        }
        *self.highlighted.lock().expect("highlight lock poisoned") = ids;
        self.dragging = dragging;
        self.active_node = active_node;
        self.displayed = displayed;
        true
    }
}

impl Render for IntersectionOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = cx.entity();
        let graph = self.graph.clone();
        let poller = canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                let changed = cx.update_entity(&overlay, |overlay, cx| {
                    let changed = overlay.sample(cx);
                    if changed {
                        cx.notify();
                    }
                    changed
                });
                if changed {
                    cx.update_entity(&graph, |_, cx| cx.notify());
                }
            },
        )
        .absolute()
        .size(px(1.0));

        let ids = self
            .displayed
            .iter()
            .map(|(id, kind)| match kind {
                IntersectionKind::Partial => id.0.to_string(),
                IntersectionKind::FullyContained => format!("{} (contained)", id.0),
            })
            .collect::<Vec<_>>();
        let status = match (self.dragging, self.active_node) {
            (Some(id), _) if ids.is_empty() => {
                format!("Dragging node {} · no intersections", id.0)
            }
            (Some(id), _) => format!("Dragging node {} · intersects: {}", id.0, ids.join(", ")),
            (None, Some(id)) if ids.is_empty() => format!("Node {} · no intersections", id.0),
            (None, Some(id)) => format!("Node {} · intersects: {}", id.0, ids.join(", ")),
            (None, None) => "Drag a node over another node".to_string(),
        };

        div().absolute().size_full().child(poller).child(
            div()
                .absolute()
                .left(px(16.0))
                .bottom(px(16.0))
                .rounded(px(7.0))
                .border(px(1.0))
                .border_color(rgb(0xd1d5db))
                .bg(rgb(0xffffff))
                .px_3()
                .py_2()
                .child(status),
        )
    }
}

struct ExampleView {
    graph: Entity<Graph>,
    overlay: Entity<IntersectionOverlay>,
}

impl Render for ExampleView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .bg(rgb(0xf7f9fb))
            .child(self.graph.clone())
            .child(self.overlay.clone())
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Intersections".into()),
                ..Default::default()
            },
            |_, cx| {
                let nodes = vec![
                    Node::new(1_u64, WorldPoint::new(25.0, 18.0))
                        .with_size(WorldSize::new(18.0, 12.0)),
                    Node::new(2_u64, WorldPoint::new(4.0, 29.0))
                        .with_size(WorldSize::new(16.0, 7.0)),
                    Node::new(3_u64, WorldPoint::new(34.0, 5.0))
                        .with_size(WorldSize::new(16.0, 7.0)),
                    Node::new(4_u64, WorldPoint::new(43.0, 29.0))
                        .with_size(WorldSize::new(7.0, 7.0)),
                ];
                let mut renderer = GraphRenderer::default();
                let highlighted = Arc::new(Mutex::new(HashMap::new()));
                renderer.register_node_type(
                    "default",
                    |node: &Node, zoom, _: &gpug::GraphStyle| NodeAppearance {
                        color: 0xffffff,
                        radius_pixels: node.size.width * zoom * 0.5,
                        shape: NodeShape::None,
                    },
                );
                let node_highlights = highlighted.clone();
                renderer.register_node_content("default", move |node: &Node, zoom| {
                    let intersection = node_highlights
                        .lock()
                        .expect("highlight lock poisoned")
                        .get(&node.id)
                        .copied();
                    let intersecting = intersection.is_some();
                    let selected = node.selected;
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(12.0))
                        .border(px(if selected { 2.0 } else { 1.0 }))
                        .border_color(rgb(if selected {
                            0x1e90ff
                        } else if intersecting {
                            0x7772b5
                        } else {
                            0xe1e3e6
                        }))
                        .bg(rgb(match intersection {
                            Some(IntersectionKind::Partial) => 0x9290c2,
                            Some(IntersectionKind::FullyContained) => 0x5956a8,
                            None => 0xffffff,
                        }))
                        .text_color(rgb(if intersecting { 0xffffff } else { 0x111111 }))
                        .text_size(px(2.0 * zoom))
                        .shadow_sm()
                        .child(format!("Node {}", node.id.0))
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, vec![]))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                let overlay = cx.new(|_| IntersectionOverlay {
                    graph: graph.clone(),
                    highlighted,
                    displayed: Vec::new(),
                    dragging: None,
                    active_node: None,
                });
                cx.new(|_| ExampleView { graph, overlay })
            },
        )
        .unwrap();
    });
}

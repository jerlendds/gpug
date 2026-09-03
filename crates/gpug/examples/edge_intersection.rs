use gpug::{
    Edge, EdgeId, Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeId, NodeShape,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, prelude::*, px, rgb, App, AppContext, Application, Context, Entity, IntoElement,
    Render, Window, WindowOptions,
};

const NODE_C: NodeId = NodeId(3);

fn node(id: u64, position: WorldPoint, caption: &str) -> Node {
    let mut node = Node::new(id, position).with_size(WorldSize::new(18.0, 7.0));
    node.metadata.insert("caption".into(), caption.into());
    node
}

fn example_data() -> GraphData {
    let mut edge = Edge::new_with_id(1_u64, 2_u64, 1_u64).with_marker_end(None);
    edge.edge_type = "bezier".into();
    GraphData::new(
        vec![
            node(NODE_C.0, WorldPoint::new(2.0, 2.0), "Node C"),
            node(1, WorldPoint::new(26.0, 2.0), "Node A"),
            node(2, WorldPoint::new(2.0, 25.0), "Node B"),
        ],
        vec![edge],
    )
}

fn candidate_edge(graph: &Graph, dragged: NodeId) -> Option<EdgeId> {
    let bounds = graph.node_bounds(dragged)?;
    graph
        .intersecting_edges(bounds)
        .into_iter()
        .filter(|id| {
            graph
                .edges()
                .iter()
                .find(|edge| edge.id == *id)
                .is_some_and(|edge| edge.source != dragged && edge.target != dragged)
        })
        .min()
}

struct EdgeIntersectionOverlay {
    graph: Entity<Graph>,
    candidate: Option<EdgeId>,
    dragged: Option<NodeId>,
}

impl EdgeIntersectionOverlay {
    fn sample(&mut self, cx: &App) -> (bool, Option<(EdgeId, NodeId)>) {
        let (dragged, candidate) = cx.read_entity(&self.graph, |graph, _| {
            let dragged = graph.nodes().iter().find_map(|node| {
                graph
                    .editor()
                    .runtimes
                    .get(&node.id)?
                    .dragging
                    .then_some(node.id)
            });
            (dragged, dragged.and_then(|id| candidate_edge(graph, id)))
        });
        let commit = match (self.dragged, dragged, self.candidate) {
            (Some(node), None, Some(edge)) => Some((edge, node)),
            _ => None,
        };
        let changed = self.dragged != dragged || self.candidate != candidate;
        self.dragged = dragged;
        self.candidate = candidate;
        (changed, commit)
    }
}

impl Render for EdgeIntersectionOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = cx.entity();
        let graph = self.graph.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                window.request_animation_frame();
                let (changed, commit) = cx.update_entity(&overlay, |overlay, cx| {
                    let result = overlay.sample(cx);
                    if result.0 {
                        cx.notify();
                    }
                    result
                });
                if changed || commit.is_some() {
                    cx.update_entity(&graph, |graph, cx| {
                        if let Some((edge, node)) = commit {
                            let next_id = graph
                                .edges()
                                .iter()
                                .map(|edge| edge.id.0)
                                .max()
                                .unwrap_or(0)
                                + 1;
                            if graph
                                .split_edge_at_node(edge, node, next_id)
                                .unwrap_or(false)
                            {
                                cx.notify();
                            }
                        } else {
                            cx.notify();
                        }
                    });
                }
            },
        )
        .absolute()
        .size_full()
    }
}

struct ExampleView {
    graph: Entity<Graph>,
    overlay: Entity<EdgeIntersectionOverlay>,
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
                app_id: Some("GPUG — Edge Intersection".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("default", |_: &Node, _: f32, _: &gpug::GraphStyle| {
                    NodeAppearance {
                        color: 0xffffff,
                        radius_pixels: 0.0,
                        shape: NodeShape::None,
                    }
                });
                renderer.register_node_content("default", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.0))
                        .border(px(if node.selected { 1.5 } else { 1.0 }))
                        .border_color(rgb(if node.selected { 0xff5eae } else { 0xe1e3e6 }))
                        .bg(rgb(0xffffff))
                        .text_color(rgb(0x111111))
                        .text_size(px(2.0 * zoom))
                        .shadow_sm()
                        .child(
                            node.metadata
                                .get("caption")
                                .cloned()
                                .unwrap_or_else(|| format!("Node {}", node.id.0)),
                        )
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(example_data())
                        .renderer(renderer)
                        .fit_on_load()
                        .show_handles(true)
                        .build(cx)
                        .unwrap()
                });
                let overlay = cx.new(|_| EdgeIntersectionOverlay {
                    graph: graph.clone(),
                    candidate: None,
                    dragged: None,
                });
                cx.new(|_| ExampleView { graph, overlay })
            },
        )
        .unwrap();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_starts_with_one_edge_and_a_detached_node() {
        let data = example_data();
        assert_eq!(data.edges.len(), 1);
        assert!(data
            .edges
            .iter()
            .all(|edge| edge.source != NODE_C && edge.target != NODE_C));
    }
}

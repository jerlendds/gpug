use gpug::{
    Edge, Graph, GraphData, GraphRenderer, Node, NodeAppearance, NodeId, NodeShape, WorldBounds,
    WorldPoint, WorldSize,
};
use gpui::{
    canvas, div, prelude::*, px, rgb, App, AppContext, Application, Context, Entity, IntoElement,
    Render, Window, WindowOptions,
};

const NODE_SIZE: WorldSize = WorldSize::new(9.0, 9.0);
const PROXIMITY_PIXELS: f32 = 80.0;

fn invisible_node(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

fn node(id: u64, position: WorldPoint, color: u32) -> Node {
    let mut node = Node::new(id, position)
        .with_size(NODE_SIZE)
        .with_type("proximity");
    node.metadata.insert("color".into(), format!("{color:06x}"));
    node
}

fn example_data() -> GraphData {
    GraphData::new(
        vec![
            node(1, WorldPoint::new(2.0, 13.0), 0x3f3f3f),
            node(2, WorldPoint::new(34.0, 8.0), 0x71b747),
            node(3, WorldPoint::new(34.0, 26.0), 0xff9500),
            node(4, WorldPoint::new(66.0, 13.0), 0x2376c9),
        ],
        vec![
            Edge::new(1_u64, 2_u64),
            Edge::new(2_u64, 4_u64),
            Edge::new(1_u64, 3_u64),
        ],
    )
}

fn center(bounds: WorldBounds) -> WorldPoint {
    WorldPoint::new(
        bounds.origin.x + bounds.size.width * 0.5,
        bounds.origin.y + bounds.size.height * 0.5,
    )
}

/// Shortest distance between the visible rectangles. Using the perimeter gap
/// instead of center distance keeps proximity intuitive for differently sized
/// nodes and at every zoom level.
fn bounds_gap(a: WorldBounds, b: WorldBounds) -> f32 {
    let a_right = a.origin.x + a.size.width;
    let a_bottom = a.origin.y + a.size.height;
    let b_right = b.origin.x + b.size.width;
    let b_bottom = b.origin.y + b.size.height;
    let dx = (a.origin.x - b_right).max(b.origin.x - a_right).max(0.0);
    let dy = (a.origin.y - b_bottom).max(b.origin.y - a_bottom).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Picks one candidate. Node id is the stable tie breaker.
fn nearest_candidate(
    dragged: NodeId,
    dragged_bounds: WorldBounds,
    nodes: impl IntoIterator<Item = (NodeId, WorldBounds, bool)>,
    existing: &[(NodeId, NodeId)],
    threshold_world: f32,
) -> Option<NodeId> {
    nodes
        .into_iter()
        .filter(|(id, _, hidden)| {
            *id != dragged
                && !hidden
                && !existing
                    .iter()
                    .any(|&(a, b)| (a == dragged && b == *id) || (a == *id && b == dragged))
        })
        .filter_map(|(id, bounds, _)| {
            let d = bounds_gap(dragged_bounds, bounds);
            (d <= threshold_world).then_some((id, d))
        })
        .min_by(|(a_id, a_distance), (b_id, b_distance)| {
            a_distance.total_cmp(b_distance).then(a_id.cmp(b_id))
        })
        .map(|(id, _)| id)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Candidate {
    source: NodeId,
    target: NodeId,
}

struct ProximityOverlay {
    graph: Entity<Graph>,
    candidate: Option<Candidate>,
    was_dragging: bool,
}

impl ProximityOverlay {
    fn sample(&mut self, cx: &App) -> (bool, Option<Candidate>) {
        let (dragging, candidate) = cx.read_entity(&self.graph, |graph, _| {
            let dragged = graph.nodes().iter().find(|node| {
                graph
                    .editor()
                    .runtimes
                    .get(&node.id)
                    .is_some_and(|runtime| runtime.dragging)
            });
            let Some(dragged) = dragged else {
                return (false, None);
            };
            let Some(dragged_bounds) = graph.node_bounds(dragged.id) else {
                return (true, None);
            };
            let nodes = graph.nodes().iter().filter_map(|node| {
                graph
                    .node_bounds(node.id)
                    .map(|bounds| (node.id, bounds, node.hidden))
            });
            let existing = graph
                .edges()
                .iter()
                .map(|edge| (edge.source, edge.target))
                .collect::<Vec<_>>();
            let candidate = nearest_candidate(
                dragged.id,
                dragged_bounds,
                nodes,
                &existing,
                PROXIMITY_PIXELS / graph.viewport().zoom(),
            )
            .map(|target| Candidate {
                source: dragged.id,
                target,
            });
            (true, candidate)
        });
        let commit = (!dragging && self.was_dragging)
            .then_some(self.candidate)
            .flatten();
        let changed = self.candidate != candidate || self.was_dragging != dragging;
        self.candidate = candidate;
        self.was_dragging = dragging;
        (changed, commit)
    }
}

impl Render for ProximityOverlay {
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
                cx.update_entity(&graph, |graph, cx| {
                    let preview = cx
                        .read_entity(&overlay, |overlay, _| overlay.candidate)
                        .and_then(|candidate| {
                            Some((
                                center(graph.node_bounds(candidate.source)?),
                                center(graph.node_bounds(candidate.target)?),
                            ))
                        });
                    graph.set_temporary_edge_preview(preview);
                    if let Some(candidate) = commit {
                        let duplicate = graph.edges().iter().any(|edge| {
                            (edge.source == candidate.source && edge.target == candidate.target)
                                || (edge.source == candidate.target
                                    && edge.target == candidate.source)
                        });
                        if !duplicate
                            && graph
                                .add_edge(Edge::new(candidate.source, candidate.target))
                                .is_ok()
                        {
                            cx.notify();
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

struct ExampleView {
    graph: Entity<Graph>,
    overlay: Entity<ProximityOverlay>,
}

impl Render for ExampleView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.graph.clone())
            .child(self.overlay.clone())
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Proximity Connect".into()),
                ..Default::default()
            },
            |_, cx| {
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("proximity", invisible_node);
                renderer.register_node_content("proximity", |node: &Node, zoom| {
                    let color = node
                        .metadata
                        .get("color")
                        .and_then(|value| u32::from_str_radix(value, 16).ok())
                        .unwrap_or(0x3f3f3f);
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border(px(if node.selected { 1.5 } else { 1.0 }))
                        .border_color(rgb(if node.selected { 0xff5eae } else { 0xe2e4e7 }))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .child(div().size(px(1.8 * zoom)).bg(rgb(color)))
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(example_data())
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                let overlay = cx.new(|_| ProximityOverlay {
                    graph: graph.clone(),
                    candidate: None,
                    was_dragging: false,
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
    fn bounds(x: f32, y: f32) -> WorldBounds {
        WorldBounds::new(WorldPoint::new(x, y), NODE_SIZE)
    }

    #[test]
    fn nearest_valid_node_wins_with_node_id_as_tie_breaker() {
        let nodes = vec![
            (NodeId(3), bounds(10.0, 0.0), false),
            (NodeId(2), bounds(-10.0, 0.0), false),
        ];
        assert_eq!(
            nearest_candidate(NodeId(1), bounds(0.0, 0.0), nodes, &[], 20.0),
            Some(NodeId(2))
        );
    }

    #[test]
    fn proximity_is_measured_between_node_perimeters() {
        let large = WorldBounds::new(WorldPoint::new(0.0, 0.0), WorldSize::new(100.0, 100.0));
        let nearby = WorldBounds::new(WorldPoint::new(140.0, 0.0), WorldSize::new(100.0, 100.0));
        assert_eq!(
            nearest_candidate(NodeId(1), large, [(NodeId(2), nearby, false)], &[], 50.0),
            Some(NodeId(2))
        );
    }

    #[test]
    fn invalid_candidates_are_ignored() {
        let nodes = vec![
            (NodeId(1), bounds(0.0, 0.0), false),
            (NodeId(2), bounds(2.0, 0.0), true),
            (NodeId(3), bounds(3.0, 0.0), false),
            (NodeId(4), bounds(100.0, 0.0), false),
        ];
        assert_eq!(
            nearest_candidate(
                NodeId(1),
                bounds(0.0, 0.0),
                nodes,
                &[(NodeId(3), NodeId(1))],
                20.0
            ),
            None
        );
    }
}

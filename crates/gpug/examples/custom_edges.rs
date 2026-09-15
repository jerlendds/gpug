#![cfg_attr(target_family = "wasm", no_main)]
//! Custom routing, an edge control, parallel edges, and a self-loop.
mod support;
use std::sync::{Arc, Mutex};

use gpug::{
    Edge, EdgeAppearance, EdgePathContext, Graph, GraphData, GraphRenderer, Node, NodeAppearance,
    NodeShape, WorldPoint, WorldSize,
};
use gpui::{div, prelude::*, px, rgb, App, AppContext, Entity, MouseButton, WindowOptions};

const BUTTON_NODE: u64 = 3;

fn quadratic(a: WorldPoint, control: WorldPoint, b: WorldPoint) -> Vec<WorldPoint> {
    (0..=24)
        .map(|index| {
            let t = index as f32 / 24.0;
            let u = 1.0 - t;
            WorldPoint::new(
                u * u * a.x + 2.0 * u * t * control.x + t * t * b.x,
                u * u * a.y + 2.0 * u * t * control.y + t * t * b.y,
            )
        })
        .collect()
}

fn custom_route(context: &EdgePathContext<'_>) -> Vec<WorldPoint> {
    let a = context.source;
    let b = context.target;
    match context.edge.metadata.get("route").map(String::as_str) {
        Some("self") => {
            // Leave and re-enter the same node, making a generous loop above it.
            let radius = 12.0;
            (0..=32)
                .map(|index| {
                    let angle =
                        std::f32::consts::FRAC_PI_2 + index as f32 / 32.0 * std::f32::consts::TAU;
                    WorldPoint::new(
                        a.x + angle.cos() * radius,
                        a.y - radius + angle.sin() * radius,
                    )
                })
                .collect()
        }
        Some("upper") | Some("lower") => {
            let sign = if context.edge.metadata.get("route").map(String::as_str) == Some("upper") {
                -1.0
            } else {
                1.0
            };
            quadratic(
                a,
                WorldPoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5 + sign * 4.5),
                b,
            )
        }
        _ => vec![a, b],
    }
}

fn card(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

fn main() {
    support::Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Custom Edges".into()),
                ..Default::default()
            },
            |_, cx| {
                let graph_slot: Arc<Mutex<Option<Entity<Graph>>>> = Arc::new(Mutex::new(None));
                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("card", card);
                renderer.register_node_type("edge-button", card);
                renderer.register_cached_node_content("card", |node: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.0 * zoom))
                        .border_1()
                        .border_color(rgb(0xd9d9d9))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px(2.5 * zoom))
                        .child(node.metadata.get("caption").cloned().unwrap_or_default())
                        .into_any_element()
                });
                let button_graph = graph_slot.clone();
                renderer.register_node_content("edge-button", move |_: &Node, zoom| {
                    let graph = button_graph.clone();
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(0xb9b9b9))
                        .text_color(rgb(0xffffff))
                        .text_size(px(2.5 * zoom))
                        .cursor_pointer()
                        .child("×")
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            let entity = graph.lock().expect("graph slot poisoned").clone();
                            if let Some(entity) = entity {
                                entity.update(cx, |graph, cx| {
                                    graph.select_node(BUTTON_NODE.into());
                                    graph.delete_selected();
                                    cx.notify();
                                });
                            }
                        })
                        .into_any_element()
                });
                renderer.register_edge_type("custom", |_: &Edge, style: &gpug::GraphStyle| {
                    EdgeAppearance {
                        color: 0xaeb3ba,
                        width_pixels: style.edge_width_pixels,
                    }
                });
                renderer.register_edge_path("custom", custom_route);

                let card_node = |id: u64, x: f32, y: f32, caption: &str| {
                    let mut node = Node::new(id, WorldPoint::new(x, y))
                        .with_size(WorldSize::new(16.0, 5.5))
                        .with_type("card");
                    node.draggable = false;
                    node.metadata.insert("caption".into(), caption.into());
                    node
                };
                let mut button = Node::new(BUTTON_NODE, WorldPoint::new(0.0, -15.0))
                    .with_size(WorldSize::new(2.2, 2.2))
                    .with_type("edge-button");
                button.draggable = false;
                button.connectable = false;

                let nodes = vec![
                    card_node(1, 0.0, -25.0, "Button Edge 1"),
                    card_node(2, 0.0, -5.0, "Button Edge 2"),
                    button,
                    card_node(4, -14.0, 8.0, "Bi Directional 1"),
                    card_node(5, 14.0, 8.0, "Bi Directional 2"),
                    card_node(6, 0.0, 28.0, "Self Connecting"),
                ];
                let mut upper = Edge::new_with_id(4_u64, 5_u64, 3_u64);
                upper.edge_type = "custom".into();
                upper.metadata.insert("route".into(), "upper".into());
                let mut lower = Edge::new_with_id(5_u64, 4_u64, 4_u64);
                lower.edge_type = "custom".into();
                lower.metadata.insert("route".into(), "lower".into());
                let mut loop_edge = Edge::new_with_id(6_u64, 6_u64, 5_u64);
                loop_edge.edge_type = "custom".into();
                loop_edge.metadata.insert("route".into(), "self".into());
                let edges = vec![
                    Edge::new_with_id(1_u64, BUTTON_NODE, 1_u64),
                    Edge::new_with_id(BUTTON_NODE, 2_u64, 2_u64),
                    upper,
                    lower,
                    loop_edge,
                ];
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(nodes, edges))
                        .renderer(renderer)
                        .interactive_layout(false)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                });
                *graph_slot.lock().expect("graph slot poisoned") = Some(graph.clone());
                graph
            },
        )
        .unwrap();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_routes_have_paintable_geometry() {
        let mut edge = Edge::new_with_id(1_u64, 2_u64, 1_u64);
        edge.metadata.insert("route".into(), "upper".into());
        let path = custom_route(&EdgePathContext {
            edge: &edge,
            source: WorldPoint::new(0.0, 0.0),
            target: WorldPoint::new(10.0, 0.0),
        });
        assert!(path.len() > 2);
        assert!(path.iter().any(|point| point.y < 0.0));
    }
}

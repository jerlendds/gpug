mod support;

use std::sync::{Arc, Mutex};

use gpug::{Edge, EdgeLabelContext, Graph, GraphData, GraphRenderer, Node, WorldPoint, WorldSize};
use gpui::{
    div, prelude::*, px, rgb, App, AppContext, Application, Entity, MouseButton, WindowOptions,
};

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Edge Toolbar".into()),
                ..Default::default()
            },
            |_, cx| {
                let graph_slot: Arc<Mutex<Option<Entity<Graph>>>> = Arc::new(Mutex::new(None));
                let mut renderer = GraphRenderer::default();
                let slot = graph_slot.clone();
                renderer.set_edge_label_renderer(move |edge: &Edge, anchors: EdgeLabelContext| {
                    let slot = slot.clone();
                    let edge_id = edge.id;
                    let zoom = anchors.zoom;
                    let width = px(12.0 * zoom);
                    let height = px(4.0 * zoom);
                    div()
                        .absolute()
                        .left(anchors.midpoint.x - width * 0.5)
                        .top(anchors.midpoint.y - height * 0.5)
                        .w(width)
                        .h(height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(2.0 * zoom))
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0x94a3b8))
                        .shadow_md()
                        .cursor_pointer()
                        .text_size(px(2.0 * zoom))
                        .child("delete edge")
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            let graph = slot.lock().expect("graph slot lock").clone();
                            if let Some(graph) = graph {
                                graph.update(cx, |graph, cx| {
                                    if graph.select_edge(edge_id) && graph.delete_selected() {
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    let mut edge = Edge::new_with_id(1_u64, 2_u64, 1_u64);
                    edge.label = Some("toolbar".into());
                    Graph::builder()
                        .data(GraphData::new(
                            vec![
                                Node::new(1_u64, WorldPoint::new(-22.0, 0.0))
                                    .with_size(WorldSize::new(12.0, 7.0)),
                                Node::new(2_u64, WorldPoint::new(22.0, 0.0))
                                    .with_size(WorldSize::new(12.0, 7.0)),
                            ],
                            vec![edge],
                        ))
                        .renderer(renderer)
                        .fit_on_load()
                        .build(cx)
                        .expect("edge-toolbar example data is valid")
                });
                *graph_slot.lock().expect("graph slot lock") = Some(graph.clone());
                cx.new(|cx| support::ExampleView::new(graph, None, false, cx))
            },
        )
        .expect("example window opens");
    });
}

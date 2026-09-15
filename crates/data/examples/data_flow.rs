//! Query an analytical graph and display its source identities as custom GPUI cards.
//! cargo run -p gpug-data --features gpug --example data_flow
use gpug::{Graph, GraphRenderer, GraphStyle, Viewport, WorldBounds, WorldPoint, WorldSize};
use gpug_data::{
    gpug_bridge,
    query::{Query, QueryBudget, QueryWorkspace},
    Edge, GraphSnapshot, Node, WorldRect,
};
use gpui::{
    div, point, prelude::*, px, rgb, App, AppContext, Application, MouseButton, WindowOptions,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
#[path = "data_flow/view.rs"]
mod view;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Source construction happens before the UI starts; interactive queries and
    // view preparation run on the dedicated worker in DataFlowView.
    let nodes = (0..6)
        .map(|id| {
            let mut node = Node::new(
                id,
                WorldRect {
                    x: (id % 3) as f64 * 280.,
                    y: (id / 3) as f64 * 190.,
                    width: 230.,
                    height: 130.,
                },
            );
            node.renderer = "flow-card".into();
            node.label =
                ["Source", "Normalize", "Match", "Enrich", "Review", "Output"][id as usize].into();
            node
        })
        .collect();
    let snapshot = GraphSnapshot::new(
        nodes,
        (0..5).map(|id| Edge::new(id + 100, id, id + 1)).collect(),
    )?;
    let snapshot = Arc::new(snapshot);
    Application::new().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG Data Flow".into()),
                ..Default::default()
            },
            move |_, cx| {
                let counters: Arc<Vec<AtomicUsize>> =
                    Arc::new((0..6).map(|_| AtomicUsize::new(0)).collect());
                let mut renderer = GraphRenderer::new(GraphStyle::default());
                renderer.register_node_content("flow-card", move |node: &gpug::Node, zoom: f32| {
                    let id = node.id.0 as usize;
                    let label = node
                        .metadata
                        .get(gpug_bridge::LABEL_KEY)
                        .cloned()
                        .unwrap_or_default();
                    let count = counters[id].load(Ordering::Relaxed);
                    let counters = Arc::clone(&counters);
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .rounded(px(10. * zoom))
                        .bg(rgb(0x172335))
                        .text_color(rgb(0xf3f6fb))
                        .border_1()
                        .border_color(rgb(0x44617d))
                        .p(px(14. * zoom))
                        .gap(px(10. * zoom))
                        .text_size(px(14. * zoom))
                        .child(format!("{}  /  {label}", node.id.0))
                        .child(
                            div()
                                .text_color(rgb(0x9cc9cf))
                                .child("Graph data → custom content"),
                        )
                        .child(
                            div()
                                .id(("run", node.id.0))
                                .cursor_pointer()
                                .rounded(px(4. * zoom))
                                .bg(rgb(0x285b69))
                                .p(px(5. * zoom))
                                .child(format!("Run step · {count}"))
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    counters[id].fetch_add(1, Ordering::Relaxed);
                                    window.refresh();
                                    cx.stop_propagation();
                                }),
                        )
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(gpug::GraphData::default())
                        .renderer(renderer)
                        .interactive_layout(false)
                        .viewport(Viewport::new(point(px(70.), px(70.)), 1.).with_max_zoom(1.25))
                        .build(cx)
                        .expect("validated demonstration graph")
                });
                cx.new(|cx| view::DataFlowView::new(graph, snapshot, cx))
            },
        )
        .expect("open GPUG data window");
    });
    Ok(())
}

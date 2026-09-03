mod support;

use gpug::{Graph, GraphData, Node, WorldPoint, WorldSize};
use gpui::{App, AppContext, Application, WindowOptions};

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Temporary Edges".into()),
                ..Default::default()
            },
            |_, cx| {
                let nodes = vec![
                    Node::new(1_u64, WorldPoint::new(-20.0, 0.0))
                        .with_size(WorldSize::new(12.0, 7.0)),
                    Node::new(2_u64, WorldPoint::new(20.0, 0.0))
                        .with_size(WorldSize::new(12.0, 7.0)),
                ];
                let graph = cx.new(|cx| {
                    let mut graph = Graph::builder()
                        .data(GraphData::new(nodes, vec![]))
                        .fit_on_load()
                        .build(cx)
                        .expect("temporary-edge example data is valid");
                    graph.set_temporary_edge_preview(Some((
                        WorldPoint::new(-14.0, 0.0),
                        WorldPoint::new(14.0, 0.0),
                    )));
                    graph
                });
                cx.new(|cx| support::ExampleView::new(graph, None, false, cx))
            },
        )
        .expect("example window opens");
    });
}

use gpug::*;
use gpui::{App, AppContext, Application, WindowOptions};

mod support;
use support::ExampleView;

/// Environment knobs, so one build can be pointed at different graph sizes and
/// level-of-detail budgets without a recompile:
///
/// - `GPUG_NODE_COUNT` node count (default 1,000)
/// - `GPUG_EDGE_PROBABILITY` small-world shortcut probability
/// - `GPUG_EDGE_BUDGET` edges drawn per frame while the layout animates
/// - `GPUG_ANIMATE=0` opens paused instead of running the layout
fn env<T: std::str::FromStr>(key: &str, fallback: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn main() {
    let node_count = env("GPUG_NODE_COUNT", 1_000usize);
    let probability = env("GPUG_EDGE_PROBABILITY", 0.00001f64);
    let edge_budget = env("GPUG_EDGE_BUDGET", 20_000usize);
    let animate = env("GPUG_ANIMATE", 1u32) != 0;
    Application::new().run(move |cx: &mut App| {
        let options = WindowOptions {
            app_id: Some("GPUG Large Graph".to_string()),
            ..Default::default()
        };
        cx.open_window(options, move |_, cx| {
            let graph = cx.new(|cx| {
                let mut data = SmallWorld::new(node_count)
                    .local_neighbors(3)
                    .shortcut_probability(probability)
                    .seed(42)
                    .generate();
                for (index, edge) in data.edges.iter_mut().enumerate() {
                    edge.id = EdgeId(index as u64 + 1);
                }
                let mut graph = Graph::builder()
                    .data(data)
                    .style(GraphStyle {
                        interactive_edge_budget: edge_budget,
                        ..GraphStyle::default()
                    })
                    .interactive_layout(animate)
                    .fit_on_load()
                    .build(cx)
                    .unwrap();
                if animate {
                    graph.start_layout();
                }
                graph
            });
            cx.new(|cx| ExampleView::new(graph, None, false, cx))
        })
        .unwrap();
    });
}

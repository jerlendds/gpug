use gpug::*;
use gpui::{App, AppContext, Application, WindowOptions};

fn main() {
    Application::new().run(|cx: &mut App| {
        let mut window_opts = WindowOptions::default();
        window_opts.app_id = Some("GPUG Kitchen Sink".to_string());

        cx.open_window(window_opts, |_, cx| {
            cx.new(|cx| {
                let node_count = 250;
                let initial_k = 3;
                let initial_beta = 0.05;
                let nodes = generate_nodes(node_count);
                let edges = generate_watts_strogatz_graph(node_count, initial_k, initial_beta);
                Graph::new(cx, nodes, edges, initial_k, initial_beta)
            })
        })
        .unwrap();
    });
}

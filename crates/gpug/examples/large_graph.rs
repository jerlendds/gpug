use gpug::*;
use gpui::{App, AppContext, Application, WindowOptions};

fn main() {
    let node_count = std::env::var("GPUG_NODE_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000);
    Application::new().run(move |cx: &mut App| {
        let options = WindowOptions {
            app_id: Some("GPUG Large Graph".to_string()),
            ..Default::default()
        };
        cx.open_window(options, move |_, cx| {
            cx.new(|cx| {
                let probability = std::env::var("GPUG_EDGE_PROBABILITY")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.00001);
                Graph::builder()
                    .data(
                        SmallWorld::new(node_count)
                            .local_neighbors(3)
                            .shortcut_probability(probability)
                            .seed(42)
                            .generate(),
                    )
                    .build(cx)
                    .unwrap()
            })
        })
        .unwrap();
    });
}

use gpui::*;
use gpui::{Application, WindowOptions};

use crate::generators::utils::generate_nodes;
use crate::generators::wattsstrogatz::generate_watts_strogatz_graph;
use crate::graph::Graph;
mod edge;
mod generators;
mod graph;
mod node;

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
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

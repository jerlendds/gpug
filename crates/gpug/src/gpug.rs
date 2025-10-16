pub use crate::generators::utils::generate_nodes;
pub use crate::generators::watts_strogatz::generate_watts_strogatz_graph;
pub use crate::graph::Graph;
pub mod edge;
pub mod generators;
pub mod graph;
pub mod node;

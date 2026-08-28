#[derive(Clone, Debug)]
pub struct GraphStyle {
    pub background: u32,
    pub node_color: u32,
    pub edge_color: u32,
    pub selection_color: u32,
    pub node_radius_world: f32,
    pub edge_width_pixels: f32,
    pub hit_radius_pixels: f32,
    pub interactive_edge_budget: usize,
}

impl Default for GraphStyle {
    fn default() -> Self {
        Self {
            background: 0xffffff,
            node_color: 0x050505,
            edge_color: 0x323232,
            selection_color: 0x1E90FF,
            node_radius_world: 2.0,
            edge_width_pixels: 0.5,
            hit_radius_pixels: 8.0,
            interactive_edge_budget: 100_000,
        }
    }
}

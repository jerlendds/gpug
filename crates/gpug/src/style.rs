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

impl GraphStyle {
    pub(crate) fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        self.node_radius_world =
            finite_non_negative_or(self.node_radius_world, defaults.node_radius_world);
        self.edge_width_pixels =
            finite_non_negative_or(self.edge_width_pixels, defaults.edge_width_pixels);
        self.hit_radius_pixels =
            finite_non_negative_or(self.hit_radius_pixels, defaults.hit_radius_pixels);
        self
    }
}

pub(crate) fn finite_non_negative_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_invalid_geometry_values() {
        let style = GraphStyle {
            node_radius_world: f32::NAN,
            edge_width_pixels: f32::INFINITY,
            hit_radius_pixels: -1.0,
            ..GraphStyle::default()
        }
        .sanitized();
        let defaults = GraphStyle::default();

        assert_eq!(style.node_radius_world, defaults.node_radius_world);
        assert_eq!(style.edge_width_pixels, defaults.edge_width_pixels);
        assert_eq!(style.hit_radius_pixels, defaults.hit_radius_pixels);
    }
}

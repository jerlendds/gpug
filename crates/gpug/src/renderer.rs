use crate::GraphStyle;

#[derive(Clone, Debug, Default)]
pub struct GraphRenderer {
    style: GraphStyle,
}

impl GraphRenderer {
    pub fn new(style: GraphStyle) -> Self {
        Self { style }
    }
    pub fn style(&self) -> &GraphStyle {
        &self.style
    }
    pub fn set_style(&mut self, style: GraphStyle) {
        self.style = style;
    }

    pub fn interactive_edge_stride(&self, edge_count: usize, active: bool) -> usize {
        if active {
            edge_count
                .div_ceil(self.style.interactive_edge_budget.max(1))
                .max(1)
        } else {
            1
        }
    }

    pub fn node_radius_pixels(&self, zoom: f32) -> f32 {
        (self.style.node_radius_world * zoom).clamp(1.0, 8.0)
    }
}

#[derive(Clone, Debug)]
pub struct GraphStyle {
    pub background: u32,
    pub node_color: u32,
    pub edge_color: u32,
    pub selection_color: u32,
    /// Preferred radius of the default circular canvas node in world units.
    /// Its screen radius is at least one pixel and three times the connection
    /// handle radius at the same zoom. Custom node renderers control their shape.
    pub node_radius_world: f32,
    /// Default rendered edge width in screen pixels.
    ///
    /// The built-in default is `2.0`. Applications may override this globally
    /// or return a different width from a registered edge-type renderer.
    pub edge_width_pixels: f32,
    pub hit_radius_pixels: f32,
    pub interactive_edge_budget: usize,
    /// Smallest on-screen node height, in pixels, that still renders the
    /// node's registered element content. Connection handles and edge markers
    /// at that node also hide below this threshold, independently of the content
    /// budget; active connection feedback remains visible.
    ///
    /// Below this the node keeps its position, size, and colors but is drawn
    /// from the scene columns as one batched quad. Element trees cost layout
    /// and text shaping per node per frame; a quad costs one instance in a
    /// buffer, and at these sizes the two are visually indistinguishable.
    pub content_lod_min_pixels: f32,
    /// Fraction of the viewport's configured maximum zoom at which registered
    /// node content becomes eligible regardless of its projected size. The
    /// content budget and visibility checks still apply. Defaults to 0.9; values
    /// outside (0, 1] fall back to that default.
    pub content_full_detail_zoom_ratio: f32,
    /// Largest number of nodes that may render element content in one frame.
    ///
    /// This bounds the worst case when a viewer zooms far enough in that many
    /// nodes clear `content_lod_min_pixels` at once.
    pub content_budget: usize,
    /// Shortest on-screen edge length, in pixels, that is still drawn.
    ///
    /// Edges shorter than this are sub-pixel scribbles between two nodes that
    /// already overlap on screen: they cost fragment bandwidth and contribute
    /// nothing a viewer can resolve.
    pub edge_lod_min_pixels: f32,
    /// Frame deadline in milliseconds that the edge level of detail is steered
    /// to hold. `None` disables the control loop and draws whatever
    /// `interactive_edge_budget` allows, at a fixed detail.
    pub frame_budget_ms: Option<f32>,
}

impl Default for GraphStyle {
    fn default() -> Self {
        Self {
            background: 0xffffff,
            node_color: 0x050505,
            edge_color: 0x323232,
            selection_color: 0x1E90FF,
            node_radius_world: 2.0,
            edge_width_pixels: 2.0,
            hit_radius_pixels: 8.0,
            interactive_edge_budget: 15_000,
            content_lod_min_pixels: 18.0,
            content_full_detail_zoom_ratio: 0.9,
            content_budget: 512,
            edge_lod_min_pixels: 2.0,
            frame_budget_ms: Some(1_000.0 / 60.0),
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
        self.content_lod_min_pixels =
            finite_non_negative_or(self.content_lod_min_pixels, defaults.content_lod_min_pixels);
        if !self.content_full_detail_zoom_ratio.is_finite()
            || self.content_full_detail_zoom_ratio <= 0.0
            || self.content_full_detail_zoom_ratio > 1.0
        {
            self.content_full_detail_zoom_ratio = defaults.content_full_detail_zoom_ratio;
        }
        self.edge_lod_min_pixels =
            finite_non_negative_or(self.edge_lod_min_pixels, defaults.edge_lod_min_pixels);
        self
    }

    pub(crate) fn content_min_pixels_at_zoom(&self, zoom: f32, max_zoom: f32) -> f32 {
        if zoom >= max_zoom * self.content_full_detail_zoom_ratio {
            0.0
        } else {
            self.content_lod_min_pixels
        }
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

    #[test]
    fn default_edges_are_two_pixels_wide() {
        assert_eq!(GraphStyle::default().edge_width_pixels, 2.0);
    }

    #[test]
    fn close_zoom_restores_content_eligibility_relative_to_configured_maximum() {
        let style = GraphStyle::default();
        for maximum in [0.5, 1.0, 4.0, 20.0] {
            assert_eq!(
                style.content_min_pixels_at_zoom(maximum * 0.89, maximum),
                18.0
            );
            assert_eq!(
                style.content_min_pixels_at_zoom(maximum * 0.9, maximum),
                0.0
            );
            assert_eq!(style.content_min_pixels_at_zoom(maximum, maximum), 0.0);
        }
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1.1] {
            let style = GraphStyle {
                content_full_detail_zoom_ratio: invalid,
                ..GraphStyle::default()
            }
            .sanitized();
            assert_eq!(style.content_full_detail_zoom_ratio, 0.9);
        }
    }
}

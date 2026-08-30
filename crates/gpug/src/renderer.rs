use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use gpui::AnyElement;

use crate::{
    Diagnostic, Edge, EdgeId, GraphStyle, Node, NodeId, NodeRuntime, SharedDiagnosticSink,
    WorldPoint, WorldSize,
};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    SelectNode { node: NodeId, multi: bool },
    MoveNode { node: NodeId, position: WorldPoint },
    ResizeNode { node: NodeId, size: WorldSize },
    SelectEdge { edge: EdgeId },
    DeleteSelection,
}
pub struct NodeRenderContext<'a> {
    pub node_id: NodeId,
    pub runtime: &'a NodeRuntime,
    actions: &'a mut Vec<EditorAction>,
}
impl NodeRenderContext<'_> {
    pub fn dispatch(&mut self, action: EditorAction) {
        self.actions.push(action)
    }
}
pub struct EdgePaintContext<'a> {
    pub edge_id: EdgeId,
    pub selected: bool,
    actions: &'a mut Vec<EditorAction>,
}
impl EdgePaintContext<'_> {
    pub fn dispatch(&mut self, action: EditorAction) {
        self.actions.push(action)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeShape {
    /// No canvas primitive. Used when registered node content paints the shell
    /// at every zoom level. Prefer `Rect` for content nodes so the graph has a
    /// cheap representation to fall back to when the node is small on screen.
    None,
    Square,
    Diamond,
    /// The node body as a rounded rectangle spanning its world size.
    ///
    /// This is the level-of-detail proxy for a node that also registers
    /// element content: it paints as a single instanced quad whose corners and
    /// border are evaluated analytically in the fragment shader, so a screen
    /// full of them costs one batch rather than one element tree each.
    Rect {
        corner_radius_world: f32,
        border_color: u32,
        border_width_pixels: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeAppearance {
    pub color: u32,
    pub radius_pixels: f32,
    pub shape: NodeShape,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeAppearance {
    pub color: u32,
    pub width_pixels: f32,
}

pub trait NodeTypeRenderer: Send + Sync {
    fn appearance(&self, node: &Node, zoom: f32, style: &GraphStyle) -> NodeAppearance;
}
pub trait EdgeTypeRenderer: Send + Sync {
    fn appearance(&self, edge: &Edge, style: &GraphStyle) -> EdgeAppearance;
}

impl<F> NodeTypeRenderer for F
where
    F: Fn(&Node, f32, &GraphStyle) -> NodeAppearance + Send + Sync,
{
    fn appearance(&self, node: &Node, zoom: f32, style: &GraphStyle) -> NodeAppearance {
        self(node, zoom, style)
    }
}
impl<F> EdgeTypeRenderer for F
where
    F: Fn(&Edge, &GraphStyle) -> EdgeAppearance + Send + Sync,
{
    fn appearance(&self, edge: &Edge, style: &GraphStyle) -> EdgeAppearance {
        self(edge, style)
    }
}

#[derive(Clone, Default)]
pub struct GraphRenderer {
    style: GraphStyle,
    node_types: Arc<HashMap<String, Arc<dyn NodeTypeRenderer>>>,
    node_contents: Arc<HashMap<String, Arc<dyn NodeContentRenderer>>>,
    cached_node_contents: Arc<HashSet<String>>,
    edge_types: Arc<HashMap<String, Arc<dyn EdgeTypeRenderer>>>,
    diagnostics: Option<SharedDiagnosticSink>,
    reported: Arc<Mutex<HashSet<String>>>,
}

pub trait NodeContentRenderer: Send + Sync {
    fn render(&self, node: &Node, zoom: f32) -> AnyElement;
}

impl<F> NodeContentRenderer for F
where
    F: Fn(&Node, f32) -> AnyElement + Send + Sync,
{
    fn render(&self, node: &Node, zoom: f32) -> AnyElement {
        self(node, zoom)
    }
}

impl fmt::Debug for GraphRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphRenderer")
            .field("style", &self.style)
            .field("node_types", &self.node_types.keys())
            .field("edge_types", &self.edge_types.keys())
            .field("diagnostics", &self.diagnostics.is_some())
            .finish()
    }
}

impl GraphRenderer {
    pub fn new(style: GraphStyle) -> Self {
        let mut renderer = Self::default();
        renderer.set_style(style);
        renderer
    }
    pub fn style(&self) -> &GraphStyle {
        &self.style
    }
    pub fn set_style(&mut self, style: GraphStyle) {
        self.style = style.sanitized();
    }
    pub fn set_diagnostic_sink(&mut self, sink: Option<SharedDiagnosticSink>) {
        self.diagnostics = sink;
    }
    fn report_once(&self, key: String, diagnostic: Diagnostic) {
        let Some(sink) = &self.diagnostics else {
            return;
        };
        if self
            .reported
            .lock()
            .expect("diagnostic lock poisoned")
            .insert(key)
        {
            sink.report(&diagnostic);
        }
    }
    pub fn register_node_type(
        &mut self,
        name: impl Into<String>,
        renderer: impl NodeTypeRenderer + 'static,
    ) {
        Arc::make_mut(&mut self.node_types).insert(name.into(), Arc::new(renderer));
    }
    /// Registers interactive content rendered inside the graph-owned shell for
    /// a node type. Positioning, visibility, selection, and dragging remain
    /// owned by `Graph`; the returned element owns only the node's contents.
    pub fn register_node_content(
        &mut self,
        name: impl Into<String>,
        renderer: impl NodeContentRenderer + 'static,
    ) {
        let name = name.into();
        Arc::make_mut(&mut self.node_contents).insert(name.clone(), Arc::new(renderer));
        Arc::make_mut(&mut self.cached_node_contents).remove(&name);
    }

    /// Registers node content whose output is derived only from the supplied
    /// node and zoom. GPUG retains the resulting view until either input
    /// changes, avoiding repeated layout and text shaping on unchanged frames.
    pub fn register_cached_node_content(
        &mut self,
        name: impl Into<String>,
        renderer: impl NodeContentRenderer + 'static,
    ) {
        let name = name.into();
        Arc::make_mut(&mut self.node_contents).insert(name.clone(), Arc::new(renderer));
        Arc::make_mut(&mut self.cached_node_contents).insert(name);
    }

    pub fn node_content(&self, node: &Node, zoom: f32) -> Option<AnyElement> {
        self.node_content_renderer(node)
            .map(|(renderer, _)| renderer)
            .map(|renderer| renderer.render(node, zoom))
    }

    pub(crate) fn node_content_renderer(
        &self,
        node: &Node,
    ) -> Option<(Arc<dyn NodeContentRenderer>, bool)> {
        if let Some(renderer) = self.node_contents.get(&node.node_type) {
            return Some((
                renderer.clone(),
                self.cached_node_contents.contains(&node.node_type),
            ));
        }
        self.node_contents.get("default").map(|renderer| {
            (
                renderer.clone(),
                self.cached_node_contents.contains("default"),
            )
        })
    }

    pub fn has_node_content(&self, node: &Node) -> bool {
        self.node_contents.contains_key(&node.node_type)
            || self.node_contents.contains_key("default")
    }
    pub fn register_edge_type(
        &mut self,
        name: impl Into<String>,
        renderer: impl EdgeTypeRenderer + 'static,
    ) {
        Arc::make_mut(&mut self.edge_types).insert(name.into(), Arc::new(renderer));
    }
    pub fn node_appearance(&self, node: &Node, zoom: f32) -> NodeAppearance {
        if !matches!(
            node.node_type.as_str(),
            "default" | "input" | "output" | "group"
        ) && !self.node_types.contains_key(&node.node_type)
        {
            self.report_once(
                format!("node:{}", node.node_type),
                Diagnostic::UnknownNodeType(node.node_type.clone()),
            );
        }
        let mut appearance = self
            .node_types
            .get(&node.node_type)
            .or_else(|| self.node_types.get("default"))
            .map_or(
                NodeAppearance {
                    color: self.style.node_color,
                    radius_pixels: self.node_radius_pixels(zoom),
                    shape: NodeShape::Square,
                },
                |renderer| renderer.appearance(node, zoom, &self.style),
            );
        appearance.radius_pixels = crate::style::finite_non_negative_or(
            appearance.radius_pixels,
            self.node_radius_pixels(zoom),
        );
        appearance
    }
    pub fn edge_appearance(&self, edge: &Edge) -> EdgeAppearance {
        if !matches!(
            edge.edge_type.as_str(),
            "default" | "straight" | "bezier" | "simplebezier" | "step" | "smoothstep"
        ) && !self.edge_types.contains_key(&edge.edge_type)
        {
            self.report_once(
                format!("edge:{}", edge.edge_type),
                Diagnostic::UnknownEdgeType(edge.edge_type.clone()),
            );
        }
        let mut appearance = self
            .edge_types
            .get(&edge.edge_type)
            .or_else(|| self.edge_types.get("default"))
            .map_or(
                EdgeAppearance {
                    color: self.style.edge_color,
                    width_pixels: self.style.edge_width_pixels,
                },
                |renderer| renderer.appearance(edge, &self.style),
            );
        appearance.width_pixels = crate::style::finite_non_negative_or(
            appearance.width_pixels,
            self.style.edge_width_pixels,
        );
        appearance
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
        let radius = self.style.node_radius_world * zoom;
        if radius.is_finite() {
            radius.clamp(1.0, 8.0)
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldPoint;
    use gpui::{div, IntoElement};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn unknown_type_falls_back_to_default_registry() {
        let mut renderer = GraphRenderer::default();
        renderer.register_node_type("default", |_: &Node, _: f32, _: &GraphStyle| {
            NodeAppearance {
                color: 7,
                radius_pixels: 3.0,
                shape: NodeShape::Diamond,
            }
        });
        let node = Node::new(1u64, WorldPoint::ZERO).with_type("unknown");
        assert_eq!(renderer.node_appearance(&node, 1.0).color, 7);
    }
    #[test]
    fn unknown_type_diagnostic_is_reported_once() {
        let reports = Arc::new(AtomicUsize::new(0));
        let counter = reports.clone();
        let mut renderer = GraphRenderer::default();
        renderer.set_diagnostic_sink(Some(Arc::new(move |_: &Diagnostic| {
            counter.fetch_add(1, Ordering::Relaxed);
        })));
        let node = Node::new(1u64, WorldPoint::ZERO).with_type("missing");
        renderer.node_appearance(&node, 1.0);
        renderer.node_appearance(&node, 1.0);
        assert_eq!(reports.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn content_caching_is_explicit_and_registration_can_change_policy() {
        let mut renderer = GraphRenderer::default();
        let node = Node::new(1u64, WorldPoint::ZERO).with_type("card");
        renderer.register_node_content("card", |_: &Node, _: f32| div().into_any_element());
        assert!(!renderer.node_content_renderer(&node).unwrap().1);

        renderer.register_cached_node_content("card", |_: &Node, _: f32| div().into_any_element());
        assert!(renderer.node_content_renderer(&node).unwrap().1);

        renderer.register_node_content("card", |_: &Node, _: f32| div().into_any_element());
        assert!(!renderer.node_content_renderer(&node).unwrap().1);
    }

    #[test]
    fn sanitizes_style_and_custom_renderer_geometry() {
        let mut renderer = GraphRenderer::new(GraphStyle {
            node_radius_world: f32::NAN,
            edge_width_pixels: -1.0,
            hit_radius_pixels: f32::INFINITY,
            ..GraphStyle::default()
        });
        let defaults = GraphStyle::default();
        assert_eq!(
            renderer.style().node_radius_world,
            defaults.node_radius_world
        );
        assert_eq!(
            renderer.style().edge_width_pixels,
            defaults.edge_width_pixels
        );
        assert_eq!(
            renderer.style().hit_radius_pixels,
            defaults.hit_radius_pixels
        );
        assert_eq!(renderer.node_radius_pixels(f32::NAN), 1.0);

        renderer.register_node_type("invalid", |_: &Node, _: f32, _: &GraphStyle| {
            NodeAppearance {
                color: 1,
                radius_pixels: f32::INFINITY,
                shape: NodeShape::Square,
            }
        });
        renderer.register_edge_type("invalid", |_: &Edge, _: &GraphStyle| EdgeAppearance {
            color: 1,
            width_pixels: f32::NAN,
        });
        let node = Node::new(1u64, WorldPoint::ZERO).with_type("invalid");
        let mut edge = Edge::new(1u64, 2u64);
        edge.edge_type = "invalid".into();

        assert_eq!(renderer.node_appearance(&node, 1.0).radius_pixels, 2.0);
        assert_eq!(
            renderer.edge_appearance(&edge).width_pixels,
            defaults.edge_width_pixels
        );
    }
}

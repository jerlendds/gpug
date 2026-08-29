mod support;

use gpug::{
    Edge, GraphData, GraphRenderer, Node, NodeAppearance, NodeResizeControl, NodeShape,
    ResizeDirection, ResizeOptions, WorldPoint, WorldSize,
};
use gpui::{div, px, rgb, IntoElement, ParentElement, Styled};

const RESIZABLE_NODE: &str = "resizable";

fn configure_renderer(renderer: &mut GraphRenderer) {
    let mut style = renderer.style().clone();
    style.selection_color = 0xff0072;
    renderer.set_style(style);
    renderer.register_node_type(RESIZABLE_NODE, |node: &Node, zoom, _: &gpug::GraphStyle| {
        NodeAppearance {
            color: 0xffffff,
            radius_pixels: node.size.width * zoom * 0.5,
            shape: NodeShape::None,
        }
    });
    renderer.register_node_content(RESIZABLE_NODE, |node: &Node, zoom: f32| {
        let caption = node
            .metadata
            .get("caption")
            .cloned()
            .unwrap_or_else(|| format!("Node {}", node.id.0));
        let custom_control = (node.id.0 == 3).then(|| {
            div()
                .absolute()
                .right(px(5.0))
                .bottom(px(3.0))
                .text_size(px((2.0 * zoom).max(16.0)))
                .text_color(rgb(0xff0072))
                .child("↘")
        });
        let selected = node.selected;
        let always_resizable = node.id.0 == 1;
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_center()
            .text_size(px((1.45 * zoom).max(11.0)))
            .rounded(px(8.0))
            .border(px(if selected || always_resizable {
                2.0
            } else {
                1.0
            }))
            .border_color(rgb(if always_resizable {
                0x4263eb
            } else if selected {
                0xff0072
            } else {
                0xd1d5db
            }))
            .bg(rgb(0xffffff))
            .shadow_sm()
            .child(caption)
            .children(custom_control)
            .into_any_element()
    });
}

fn example_data() -> GraphData {
    let mut standard = Node::new(1u64, WorldPoint::new(0.0, -10.0))
        .with_size(WorldSize::new(20.0, 8.0))
        .with_type(RESIZABLE_NODE)
        .with_always_visible_resize_controls()
        .with_resize_control_color(0x4263eb);
    standard
        .metadata
        .insert("caption".into(), "NodeResizer".into());

    let mut selected = Node::new(2u64, WorldPoint::new(-18.0, 8.0))
        .with_size(WorldSize::new(13.0, 20.0))
        .with_type(RESIZABLE_NODE)
        .with_resize_directions([
            ResizeDirection::NorthWest,
            ResizeDirection::NorthEast,
            ResizeDirection::SouthEast,
            ResizeDirection::SouthWest,
        ]);
    selected
        .metadata
        .insert("caption".into(), "NodeResizer\nwhen\nselected".into());

    let mut custom = Node::new(3u64, WorldPoint::new(17.0, 8.0))
        .with_size(WorldSize::new(19.0, 8.0))
        .with_type(RESIZABLE_NODE)
        .with_resize_directions([ResizeDirection::SouthEast])
        .with_custom_resize_controls()
        .with_resize_control_hit_radius(36.0);
    custom
        .metadata
        .insert("caption".into(), "Custom Resize Icon  ↘".into());

    GraphData::new(
        vec![standard, selected, custom],
        vec![Edge::new(2u64, 3u64)],
    )
}

// A custom UI element can keep this control as its gesture state and pass
// pointer positions to Graph::{begin,update,end}_node_resize.
#[allow(dead_code)]
fn custom_resize_icon_control() -> NodeResizeControl {
    NodeResizeControl::new(3u64, ResizeDirection::SouthEast).with_options(ResizeOptions {
        min: WorldSize::new(8.0, 5.0),
        max: WorldSize::new(32.0, 20.0),
        ..ResizeOptions::default()
    })
}

catalog_example!(support::CatalogExample::new(
    "Node Resizer",
    "Drag any selected node's edge or corner handles."
)
.node_count(3)
.show_resize_handles(true)
.configure_renderer(configure_renderer)
.initial_data(example_data));

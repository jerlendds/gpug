#![cfg_attr(target_family = "wasm", no_main)]
mod support;
use gpug::{
    ConnectionLineAppearance, ConnectionLineContext, Graph, GraphData, GraphRenderer, Node,
    NodeAppearance, NodeShape, WorldPoint, WorldSize,
};
use gpui::{div, px, rgb, App, AppContext, IntoElement, ParentElement, Styled, WindowOptions};

const NODE_SIZE: WorldSize = WorldSize::new(34.0, 22.0);

fn card(_: &Node, _: f32, _: &gpug::GraphStyle) -> NodeAppearance {
    NodeAppearance {
        color: 0xffffff,
        radius_pixels: 0.0,
        shape: NodeShape::None,
    }
}

/// A connection-line renderer controls only presentation. GPUG continues to
/// own the gesture, validation, and snapping to nearby compatible handles.
fn dashed_connection(context: &ConnectionLineContext<'_>) -> ConnectionLineAppearance {
    ConnectionLineAppearance {
        color: if context.valid == Some(false) {
            0xef4444
        } else {
            0x1428f5
        },
        width_pixels: 3.0,
        dash_pixels: 8.0,
        gap_pixels: 7.0,
        end_radius_pixels: 7.0,
        bend_pixels: 70.0,
    }
}

fn main() {
    support::Application::new().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG — Custom Connection Line".into()),
                ..Default::default()
            },
            |_, cx| {
                let node = Node::new(1_u64, WorldPoint::ZERO)
                    .with_size(NODE_SIZE)
                    .with_type("card");

                let mut renderer = GraphRenderer::default();
                renderer.register_node_type("card", card);
                renderer.register_cached_node_content("card", |_: &Node, zoom| {
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(2.5 * zoom))
                        .border(px(1.5))
                        .border_color(rgb(0xff5eae))
                        .bg(rgb(0xffffff))
                        .shadow_sm()
                        .text_size(px(3.0 * zoom))
                        .child("Node")
                        .into_any_element()
                });
                renderer.set_connection_line_renderer(dashed_connection);

                cx.new(|cx| {
                    Graph::builder()
                        .data(GraphData::new(vec![node], vec![]))
                        .renderer(renderer)
                        .show_handles(true)
                        .fit_on_load()
                        .build(cx)
                        .unwrap()
                })
            },
        )
        .unwrap();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpug::{HandleKey, HandleKind, NodeId};

    #[test]
    fn invalid_targets_get_the_rejected_color() {
        let key = HandleKey {
            node: NodeId(1),
            id: None,
            kind: HandleKind::Source,
        };
        assert_eq!(
            dashed_connection(&ConnectionLineContext {
                from: &key,
                valid: Some(false)
            })
            .color,
            0xef4444
        );
    }
}

#![cfg_attr(target_family = "wasm", no_main)]
mod support;

use gpug::{ConnectionLineAppearance, GraphRenderer};

fn renderer(renderer: &mut GraphRenderer) {
    renderer.set_connection_line_renderer(|context: &gpug::ConnectionLineContext<'_>| {
        ConnectionLineAppearance {
            color: if context.valid == Some(false) {
                0xef4444
            } else {
                0x8b5cf6
            },
            width_pixels: 4.0,
            dash_pixels: 8.0,
            gap_pixels: 5.0,
            end_radius_pixels: 6.0,
            bend_pixels: 34.0,
        }
    });
}

catalog_example!(support::CatalogExample::new(
    "Multi Connection Line",
    "Every connection gesture uses a curved, dashed preview with a drop indicator."
)
.show_handles(true)
.configure_renderer(renderer));

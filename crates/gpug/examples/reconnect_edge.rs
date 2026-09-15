#![cfg_attr(target_family = "wasm", no_main)]
mod support;

catalog_example!(support::CatalogExample::new(
    "Reconnect Edge",
    "Select an edge and drag either endpoint handle to another node handle."
)
.node_count(4)
.show_handles(true));

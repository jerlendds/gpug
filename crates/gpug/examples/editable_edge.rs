#![cfg_attr(target_family = "wasm", no_main)]
mod support;

catalog_example!(support::CatalogExample::new(
    "Editable Edge",
    "Select an edge, reconnect either endpoint, or press Delete to remove it."
)
.node_count(4)
.show_handles(true));

mod support;

catalog_example!(support::CatalogExample::new(
    "Edge Types",
    "Straight, Bezier, step, and smooth-step routing share selection and hit testing."
)
.node_count(7)
.edge_types(&["straight", "bezier", "step", "smoothstep"]));

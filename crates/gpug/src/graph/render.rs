use super::*;

impl Render for Graph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _frame = profile::scope(Phase::Render);
        let data_updates = self.data_api.take_pending();
        if !data_updates.is_empty() {
            let changes = data_updates
                .into_iter()
                .filter_map(|(id, patch)| {
                    let mut node = self.model.node(id)?.clone();
                    node.metadata.extend(patch);
                    Some(NodeChange::Replace { id, item: node })
                })
                .collect::<Vec<_>>();
            self.model
                .emit_nodes(changes)
                .expect("graph data API produced valid metadata-only node updates");
            self.flush();
        }
        let revisions = self.model.store.dirty.revisions;
        let data_api_revision = (
            revisions.membership,
            revisions.node_specs,
            revisions.edge_specs,
            revisions.nodes,
            revisions.selection,
        );
        if self.data_api.has_external_consumer() && self.data_api_sync_revision != data_api_revision
        {
            self.data_api.sync(&self.model.nodes, &self.model.edges);
            self.data_api_sync_revision = data_api_revision;
        }
        {
            let _scope = profile::scope(Phase::Sync);
            self.sync();
        }
        if self.fit_on_load_pending {
            if let Some(size) = self.canvas_size() {
                self.fit_to_view(size, px(40.0));
                self.fit_on_load_pending = false;
            }
        }
        let viewport = self.viewport;
        let renderer = self.renderer.clone();
        let style = renderer.style().clone();
        let _cull = profile::scope(Phase::Cull);
        let culled = if self.only_render_visible_elements {
            let size = self.canvas_size().unwrap_or_else(|| window.viewport_size());
            let a = self.viewport.screen_to_world(point(px(0.0), px(0.0)));
            let b = self
                .viewport
                .screen_to_world(point(size.width, size.height));
            let camera = WorldBounds::new(
                WorldPoint::new(a.x.min(b.x), a.y.min(b.y)),
                crate::WorldSize::new((a.x - b.x).abs(), (a.y - b.y).abs()),
            );
            // Taken out and put back so the cull can borrow the store's
            // columns and its index at the same time.
            let mut indices = std::mem::take(&mut self.visible_indices);
            let mut flags = std::mem::take(&mut self.visible_flags);
            let store = &mut self.model.store;
            store.visibility.cull(
                &store.columns,
                camera,
                Rc::make_mut(&mut indices),
                Rc::make_mut(&mut flags),
            );
            self.visible_indices = indices;
            self.visible_flags = flags;
            true
        } else {
            false
        };
        let visible_nodes = culled.then(|| self.visible_flags.clone());
        drop(_cull);
        {
            let _scope = profile::scope(Phase::Scene);
            self.refresh_scene(&renderer, viewport.zoom());
        }
        let hidden = self.scene.hidden.clone();
        let positions = self.scene.positions.clone();
        let node_sizes = self.scene.node_sizes.clone();
        let node_appearances = self.scene.node_appearances.clone();
        let edge_kinds = self.scene.edge_kinds.clone();
        let edge_ids = self.scene.edge_ids.clone();
        let edge_appearances = self.scene.edge_appearances.clone();
        let edge_markers = self.scene.edge_markers.clone();
        let edge_geometries = self.scene.edge_geometries.clone();
        let selected = self.scene.selected.clone();
        let show_handles = self.show_handles;
        let target_handle_position = self.target_handle_position;
        let source_handle_position = self.source_handle_position;
        let show_resize_handles = self.show_resize_handles;
        let resize_directions = self.scene.resize_directions.clone();
        let show_resize_controls = self.scene.show_resize_controls.clone();
        let resize_controls_always_visible = self.scene.resize_controls_always_visible.clone();
        let resize_control_colors = self.scene.resize_control_colors.clone();
        let selected_edges = self.scene.selected_edges.clone();
        let edges = self.layout_edges.clone();
        let multi_selection = selected_node_bounds(&selected, &positions, &node_sizes, &hidden);
        // Fold the frame that just elapsed into the detail control loop, then
        // draw this one at whatever detail the measurement says fits. The
        // static budget stays the ceiling; the governor only ever removes
        // more, so an application that asked for a modest budget keeps it.
        //
        // Only frames from a continuously rendering graph are measured. At
        // rest the graph renders when something asks it to, so the gap between
        // two renders is idle time, not a frame that ran long; feeding those
        // gaps to the loop makes an untouched graph shed detail while it has
        // nothing to keep up with.
        let governor_stride = match style.frame_budget_ms {
            Some(target_ms) if self.is_rendering_continuously() => {
                let now = std::time::Instant::now();
                if let Some(previous) = self.last_frame.replace(now) {
                    self.last_frame_ms = now.duration_since(previous).as_secs_f32() * 1_000.0;
                    self.governor.observe(self.last_frame_ms, target_ms);
                }
                self.governor.stride_for(edges.len())
            }
            _ => {
                self.last_frame = None;
                self.last_frame_ms = 0.0;
                self.governor.relax();
                1
            }
        };
        let edge_stride = renderer
            .interactive_edge_stride(edges.len(), self.playing)
            .max(governor_stride);
        let edge_lod_min_pixels = style.edge_lod_min_pixels;
        let reconnecting_edge = reconnecting_edge_id(&self.connection.state);
        let temporary_edge_preview = self.temporary_edge_preview;
        let default_edge = EdgeAppearance {
            color: style.edge_color,
            width_pixels: style.edge_width_pixels,
        };
        let marquee = self.selection_start.zip(self.selection_current);
        let connection_line = match &self.connection.state {
            ConnectionState::Connecting {
                from,
                pointer,
                valid,
                ..
            } => self.model.node(from.node).map(|node| {
                let center = self.world_to_screen(self.node_center(node));
                let origin = self.screen_to_world(connection_handle_position(
                    center,
                    self.renderer
                        .node_appearance(node, self.viewport.zoom())
                        .radius_pixels,
                    from.kind,
                    self.target_handle_position,
                    self.source_handle_position,
                    self.viewport.zoom(),
                ));
                let appearance =
                    renderer.connection_line_appearance(&crate::ConnectionLineContext {
                        from,
                        valid: *valid,
                    });
                (origin, *pointer, appearance)
            }),
            _ => None,
        };

        let revisions = self.model.store.dirty.revisions;
        let viewport_size = self.canvas_size().unwrap_or_else(|| window.viewport_size());
        let content_revision = (
            revisions.membership,
            revisions.node_specs,
            revisions.nodes,
            revisions.selection,
            revisions.viewport,
            viewport.zoom().to_bits(),
            (viewport_size.width / px(1.0)).to_bits(),
            (viewport_size.height / px(1.0)).to_bits(),
            (self.canvas_origin().x / px(1.0)).to_bits(),
            (self.canvas_origin().y / px(1.0)).to_bits(),
        );
        // Level-of-detail selection for element content. A node renders its
        // registered element tree only while it is large enough on screen for
        // that detail to be legible, and only while the frame's budget lasts.
        // Every other node keeps its position, size, and colors and is drawn
        // from the scene columns instead, so nothing disappears - the
        // representation gets cheaper, the graph does not get emptier.
        let content_present = self.content_present.clone();
        let node_order = self.scene.node_order.clone();
        select_content_lod(
            &node_order,
            &hidden,
            &content_present,
            visible_nodes.as_deref().map(Vec::as_slice),
            &node_sizes,
            viewport.zoom(),
            style.content_lod_min_pixels,
            style.content_budget,
            Rc::make_mut(&mut self.content_drawn),
        );
        let content_drawn = self.content_drawn.clone();
        let has_content = content_present.clone();

        let mut uncached_node_contents = Vec::new();
        for index in node_order.iter().copied() {
            if !content_drawn[index] {
                continue;
            }
            let node = &self.model.nodes[index];
            let Some((content_renderer, cached)) = renderer.node_content_renderer(node) else {
                continue;
            };
            if cached {
                continue;
            }
            let center = viewport.world_to_screen(positions[index]);
            let width = px((node_sizes[index].width * viewport.zoom()).max(1.0));
            let height = px((node_sizes[index].height * viewport.zoom()).max(1.0));
            uncached_node_contents.push(
                div()
                    .absolute()
                    .left(center.x - width * 0.5)
                    .top(center.y - height * 0.5)
                    .w(width)
                    .h(height)
                    .child(content_renderer.render(node, viewport.zoom())),
            );
        }
        let _content = profile::scope(Phase::Content);
        if self.node_content_layer_revision != Some(content_revision) {
            self.node_content_cache
                .retain(|id, _| self.model.store.node_lookup.contains_key(id));
            let mut items = Vec::new();
            for index in node_order.iter().copied() {
                if !content_drawn[index] {
                    continue;
                }
                let node = &self.model.nodes[index];
                let Some((content_renderer, cached)) = renderer.node_content_renderer(node) else {
                    continue;
                };
                if !cached {
                    continue;
                }
                let content = if let Some(content) = self.node_content_cache.get(&node.id).cloned()
                {
                    let node = node.clone();
                    cx.update_entity(&content, |cached, cx| {
                        if cached.node != node
                            || cached.zoom.to_bits() != viewport.zoom().to_bits()
                            || !Arc::ptr_eq(&cached.renderer, &content_renderer)
                        {
                            cached.node = node;
                            cached.zoom = viewport.zoom();
                            cached.renderer = content_renderer;
                            cx.notify();
                        }
                    });
                    content
                } else {
                    let content = cx.new(|_| CachedNodeContent {
                        renderer: content_renderer,
                        node: node.clone(),
                        zoom: viewport.zoom(),
                    });
                    self.node_content_cache.insert(node.id, content.clone());
                    content
                };
                items.push(NodeContentItem {
                    center: viewport.world_to_screen(positions[index]),
                    size: size(
                        px((node_sizes[index].width * viewport.zoom()).max(1.0)),
                        px((node_sizes[index].height * viewport.zoom()).max(1.0)),
                    ),
                    content,
                });
            }
            cx.update_entity(&self.node_content_layer, |layer, cx| {
                layer.items = items;
                cx.notify();
            });
            self.node_content_layer_revision = Some(content_revision);
        }
        drop(_content);
        profile::count(
            Counter::VisibleNodes,
            if culled {
                self.visible_indices.len()
            } else {
                self.model.nodes.len()
            },
        );
        if profile::enabled() {
            profile::count(
                Counter::ContentNodes,
                self.content_drawn.iter().filter(|drawn| **drawn).count(),
            );
        }
        profile::frame(
            self.model.nodes.len(),
            self.model.edges.len(),
            if culled {
                self.visible_indices.len()
            } else {
                self.model.nodes.len()
            },
            self.node_content_cache.len(),
        );
        let node_content_layer = self.node_content_layer.clone();
        if self.data_api.has_pending() {
            cx.notify();
        }

        let graph_handle = cx.entity();
        let bounds_handle = graph_handle.clone();
        let paint_viewport = self.window_viewport();
        let graph_canvas = canvas(
            move |bounds, _window, cx| {
                cx.update_entity(&bounds_handle, |graph, cx| {
                    if graph.canvas_bounds != Some(bounds) {
                        graph.canvas_bounds = Some(bounds);
                        graph.model.store.dirty.mark_viewport();
                        cx.notify();
                    }
                });
            },
            move |bounds, _, window, _cx| {
                let viewport = paint_viewport;
                let _scope = profile::scope(Phase::Paint);
                let st = (point(0., 1.), point(0., 1.), point(0., 1.));
                // Every primitive is submitted inside a layer.
                //
                // Outside one, gpui derives each primitive's draw order by
                // inserting its bounds into a tree and testing it against
                // everything already there, which is a per-primitive cost that
                // a graph pays thousands of times a frame. A layer resolves
                // that order once and hands it to everything inside, so
                // submitting a node body becomes a push onto a vector.
                //
                // One layer per pass, in back-to-front order, is also what
                // keeps edges behind nodes and overlays in front without
                // sorting anything: the passes are the depth order.
                window.paint_layer(bounds, |window| {
                    let _scope = profile::scope(Phase::PaintEdges);
                    let mut edge_path = Path::new(point(px(0.0), px(0.0)));
                    for (edge_index, edge) in edges.iter().enumerate().step_by(edge_stride) {
                        if reconnecting_edge == Some(edge_ids[edge_index]) {
                            continue;
                        }
                        if hidden[edge.source] || hidden[edge.target] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible[edge.source] && !visible[edge.target])
                        {
                            continue;
                        }
                        if edge_appearances[edge_index] != default_edge
                            || edge_kinds[edge_index] != EdgeKind::Straight
                        {
                            continue;
                        }
                        let p1 = viewport.world_to_screen(positions[edge.source]);
                        let p2 = viewport.world_to_screen(positions[edge.target]);
                        if (p1.x < bounds.left() && p2.x < bounds.left())
                            || (p1.x > bounds.right() && p2.x > bounds.right())
                            || (p1.y < bounds.top() && p2.y < bounds.top())
                            || (p1.y > bounds.bottom() && p2.y > bounds.bottom())
                        {
                            profile::count(Counter::RejectedEdges, 1);
                            continue;
                        }
                        let direction = point(p2.x - p1.x, p2.y - p1.y);
                        let length = direction.magnitude() as f32;
                        // Level-of-detail rejection. An edge shorter than a
                        // couple of pixels connects two nodes that already
                        // overlap on screen: it costs two triangles and a band
                        // of fragments to communicate a relationship the
                        // viewer cannot separate from the nodes themselves.
                        if length <= edge_lod_min_pixels {
                            profile::count(Counter::RejectedEdges, 1);
                            continue;
                        }
                        let normal = point(-direction.y, direction.x)
                            * (style.edge_width_pixels.max(0.25) * 0.5 / length);
                        let p1a = point(p1.x + normal.x, p1.y + normal.y);
                        let p1b = point(p1.x - normal.x, p1.y - normal.y);
                        let p2a = point(p2.x + normal.x, p2.y + normal.y);
                        let p2b = point(p2.x - normal.x, p2.y - normal.y);
                        edge_path.push_triangle((p1a, p1b, p2a), st);
                        edge_path.push_triangle((p2a, p1b, p2b), st);
                        profile::count(Counter::VisibleEdges, 1);
                        profile::count(Counter::Triangles, 2);
                    }
                    window.paint_path(edge_path, rgba((style.edge_color << 8) | 0x30));
                    profile::count(Counter::DrawCalls, 1);

                    if let Some((from, to)) = temporary_edge_preview {
                        let from = viewport.world_to_screen(from);
                        let to = viewport.world_to_screen(to);
                        let delta = point(to.x - from.x, to.y - from.y);
                        let length = delta.magnitude() as f32;
                        let dots = (length / 10.0).floor().max(2.0) as usize;
                        for index in 0..=dots {
                            let t = index as f32 / dots as f32;
                            let center = point(from.x + delta.x * t, from.y + delta.y * t);
                            window.paint_quad(fill(
                                Bounds::centered_at(center, size(px(3.0), px(3.0))),
                                rgba(0x7f8792d9),
                            ));
                        }
                    }

                    for (edge_index, edge) in edges.iter().enumerate().step_by(edge_stride) {
                        if reconnecting_edge == Some(edge_ids[edge_index]) {
                            continue;
                        }
                        if hidden[edge.source] || hidden[edge.target] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible[edge.source] && !visible[edge.target])
                        {
                            continue;
                        }
                        let appearance = edge_appearances[edge_index];
                        let kind = edge_kinds[edge_index];
                        if appearance == default_edge && kind == EdgeKind::Straight {
                            continue;
                        }
                        // Straight edges carry no cached geometry; a custom
                        // appearance on one still has to be drawn here, from its
                        // endpoints.
                        let straight;
                        let world_points = match edge_geometries
                            .get(edge_index)
                            .and_then(|geometry| geometry.as_ref())
                        {
                            Some(points) => points.as_slice(),
                            None => {
                                straight = [positions[edge.source], positions[edge.target]];
                                &straight[..]
                            }
                        };
                        let mut path = Path::new(viewport.world_to_screen(world_points[0]));
                        for pair in world_points.windows(2) {
                            let p1 = viewport.world_to_screen(pair[0]);
                            let p2 = viewport.world_to_screen(pair[1]);
                            let direction = point(p2.x - p1.x, p2.y - p1.y);
                            let length = direction.magnitude() as f32;
                            if length <= 0.0001 {
                                continue;
                            }
                            let normal = point(-direction.y, direction.x)
                                * (appearance.width_pixels.max(0.25) * 0.5 / length);
                            path.push_triangle(
                                (
                                    point(p1.x + normal.x, p1.y + normal.y),
                                    point(p1.x - normal.x, p1.y - normal.y),
                                    point(p2.x + normal.x, p2.y + normal.y),
                                ),
                                st,
                            );
                            path.push_triangle(
                                (
                                    point(p2.x + normal.x, p2.y + normal.y),
                                    point(p1.x - normal.x, p1.y - normal.y),
                                    point(p2.x - normal.x, p2.y - normal.y),
                                ),
                                st,
                            );
                        }
                        window.paint_path(path, rgba((appearance.color << 8) | 0xff));
                    }

                    if !selected_edges.is_empty() {
                        let mut path = Path::new(point(px(0.0), px(0.0)));
                        for (edge_index, edge) in selected_edges.iter().copied() {
                            if reconnecting_edge == Some(edge_ids[edge_index]) {
                                continue;
                            }
                            if hidden[edge.source] || hidden[edge.target] {
                                continue;
                            }
                            if visible_nodes.as_ref().is_some_and(|visible| {
                                !visible[edge.source] && !visible[edge.target]
                            }) {
                                continue;
                            }
                            let straight;
                            let world_points = match edge_geometries
                                .get(edge_index)
                                .and_then(|geometry| geometry.as_ref())
                            {
                                Some(points) => points.as_slice(),
                                None => {
                                    straight = [positions[edge.source], positions[edge.target]];
                                    &straight[..]
                                }
                            };
                            if world_points.len() < 2 {
                                continue;
                            }
                            for pair in world_points.windows(2) {
                                let p1 = viewport.world_to_screen(pair[0]);
                                let p2 = viewport.world_to_screen(pair[1]);
                                let direction = point(p2.x - p1.x, p2.y - p1.y);
                                let length = direction.magnitude() as f32;
                                if length <= 0.0001 {
                                    continue;
                                }
                                let normal = point(-direction.y, direction.x) * (2.0 / length);
                                path.push_triangle(
                                    (
                                        point(p1.x + normal.x, p1.y + normal.y),
                                        point(p1.x - normal.x, p1.y - normal.y),
                                        point(p2.x + normal.x, p2.y + normal.y),
                                    ),
                                    st,
                                );
                                path.push_triangle(
                                    (
                                        point(p2.x + normal.x, p2.y + normal.y),
                                        point(p1.x - normal.x, p1.y - normal.y),
                                        point(p2.x - normal.x, p2.y - normal.y),
                                    ),
                                    st,
                                );
                            }
                            let endpoint_size = px(RECONNECT_HANDLE_SIZE_WORLD * viewport.zoom());
                            let endpoints = [
                                viewport.world_to_screen(world_points[0]),
                                viewport.world_to_screen(world_points[world_points.len() - 1]),
                            ];
                            for endpoint in endpoints {
                                window.paint_quad(fill(
                                    Bounds::centered_at(
                                        endpoint,
                                        size(endpoint_size, endpoint_size),
                                    ),
                                    rgb(0xffffff),
                                ));
                                window.paint_quad(outline(
                                    Bounds::centered_at(
                                        endpoint,
                                        size(endpoint_size, endpoint_size),
                                    ),
                                    rgb(style.selection_color),
                                    BorderStyle::default(),
                                ));
                            }
                        }
                        window.paint_path(path, rgb(style.selection_color));
                    }

                    let mut marker_path = Path::new(point(px(0.0), px(0.0)));
                    for (index, edge) in edges.iter().enumerate().step_by(edge_stride) {
                        if reconnecting_edge == Some(edge_ids[index]) {
                            continue;
                        }
                        if hidden[edge.source] || hidden[edge.target] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible[edge.source] && !visible[edge.target])
                        {
                            continue;
                        }
                        let (start_marker, end_marker) = &edge_markers[index];
                        if start_marker.is_none() && end_marker.is_none() {
                            continue;
                        }
                        let straight;
                        let world_points = match edge_geometries
                            .get(index)
                            .and_then(|geometry| geometry.as_ref())
                        {
                            Some(points) => points.as_slice(),
                            None => {
                                straight = [positions[edge.source], positions[edge.target]];
                                &straight[..]
                            }
                        };
                        let Some((start_next, start)) = world_points
                            .windows(2)
                            .find_map(|pair| (pair[0] != pair[1]).then_some((pair[1], pair[0])))
                        else {
                            continue;
                        };
                        let Some((end_previous, end)) = world_points
                            .windows(2)
                            .rev()
                            .find_map(|pair| (pair[0] != pair[1]).then_some((pair[0], pair[1])))
                        else {
                            continue;
                        };
                        let appearance = edge_appearances[index];
                        let mut append_marker =
                            |marker: &EdgeMarker,
                             tip_world: WorldPoint,
                             previous_world: WorldPoint| {
                                let tip = viewport.world_to_screen(tip_world);
                                let previous = viewport.world_to_screen(previous_world);
                                let Some((tip, left, right)) =
                                    marker_triangle(tip, previous, appearance.width_pixels)
                                else {
                                    return;
                                };
                                match marker {
                                    EdgeMarker::Arrow => {
                                        let stroke = appearance.width_pixels.max(1.0);
                                        for endpoint in [left, right] {
                                            let direction =
                                                point(endpoint.x - tip.x, endpoint.y - tip.y);
                                            let segment_length = direction.magnitude() as f32;
                                            if segment_length <= 0.0001 {
                                                continue;
                                            }
                                            let side = point(-direction.y, direction.x)
                                                * (stroke * 0.5 / segment_length);
                                            marker_path.push_triangle(
                                                (
                                                    point(tip.x + side.x, tip.y + side.y),
                                                    point(tip.x - side.x, tip.y - side.y),
                                                    point(endpoint.x + side.x, endpoint.y + side.y),
                                                ),
                                                st,
                                            );
                                            marker_path.push_triangle(
                                                (
                                                    point(endpoint.x + side.x, endpoint.y + side.y),
                                                    point(tip.x - side.x, tip.y - side.y),
                                                    point(endpoint.x - side.x, endpoint.y - side.y),
                                                ),
                                                st,
                                            );
                                        }
                                    }
                                    EdgeMarker::ArrowClosed | EdgeMarker::Custom(_) => {
                                        marker_path.push_triangle((tip, left, right), st);
                                    }
                                }
                            };
                        if let Some(marker) = end_marker {
                            append_marker(marker, end, end_previous);
                        }
                        if let Some(marker) = start_marker {
                            append_marker(marker, start, start_next);
                        }
                    }
                    window.paint_path(marker_path, rgb(style.edge_color));
                    profile::count(Counter::DrawCalls, 1);
                });

                window.paint_layer(bounds, |window| {
                    let _scope = profile::scope(Phase::PaintNodes);

                    if let Some((a, b, appearance)) = connection_line {
                        let p1 = viewport.world_to_screen(a);
                        let p2 = viewport.world_to_screen(b);
                        let bend = px(appearance.bend_pixels.copysign((p2.y - p1.y) / px(1.0)));
                        let c1 = point(p1.x, p1.y + bend);
                        let c2 = point(p1.x, p2.y);
                        let steps = if appearance.bend_pixels == 0.0 { 1 } else { 48 };
                        let mut samples = Vec::with_capacity(steps + 1);
                        for index in 0..=steps {
                            let t = index as f32 / steps as f32;
                            let u = 1.0 - t;
                            samples.push(point(
                                p1.x * (u * u * u)
                                    + c1.x * (3.0 * u * u * t)
                                    + c2.x * (3.0 * u * t * t)
                                    + p2.x * (t * t * t),
                                p1.y * (u * u * u)
                                    + c1.y * (3.0 * u * u * t)
                                    + c2.y * (3.0 * u * t * t)
                                    + p2.y * (t * t * t),
                            ));
                        }
                        let mut path = Path::new(p1);
                        let mut distance = 0.0_f32;
                        let period = appearance.dash_pixels + appearance.gap_pixels;
                        for pair in samples.windows(2) {
                            let direction = point(pair[1].x - pair[0].x, pair[1].y - pair[0].y);
                            let length = direction.magnitude() as f32;
                            if length <= 0.0001 {
                                continue;
                            }
                            let painted = appearance.dash_pixels <= 0.0
                                || period <= 0.0
                                || distance % period < appearance.dash_pixels;
                            distance += length;
                            if !painted {
                                continue;
                            }
                            let normal = point(-direction.y, direction.x)
                                * (appearance.width_pixels * 0.5 / length);
                            path.push_triangle(
                                (
                                    point(pair[0].x + normal.x, pair[0].y + normal.y),
                                    point(pair[0].x - normal.x, pair[0].y - normal.y),
                                    point(pair[1].x + normal.x, pair[1].y + normal.y),
                                ),
                                st,
                            );
                            path.push_triangle(
                                (
                                    point(pair[1].x + normal.x, pair[1].y + normal.y),
                                    point(pair[0].x - normal.x, pair[0].y - normal.y),
                                    point(pair[1].x - normal.x, pair[1].y - normal.y),
                                ),
                                st,
                            );
                        }
                        if appearance.end_radius_pixels > 0.0 {
                            let outer = appearance.end_radius_pixels;
                            let inner = (outer - appearance.width_pixels).max(0.0);
                            for index in 0..16 {
                                let a = std::f32::consts::TAU * index as f32 / 16.0;
                                let b = std::f32::consts::TAU * (index + 1) as f32 / 16.0;
                                path.push_triangle(
                                    (
                                        point(
                                            p2.x + px(a.cos() * outer),
                                            p2.y + px(a.sin() * outer),
                                        ),
                                        point(
                                            p2.x + px(a.cos() * inner),
                                            p2.y + px(a.sin() * inner),
                                        ),
                                        point(
                                            p2.x + px(b.cos() * outer),
                                            p2.y + px(b.sin() * outer),
                                        ),
                                    ),
                                    st,
                                );
                                path.push_triangle(
                                    (
                                        point(
                                            p2.x + px(b.cos() * outer),
                                            p2.y + px(b.sin() * outer),
                                        ),
                                        point(
                                            p2.x + px(a.cos() * inner),
                                            p2.y + px(a.sin() * inner),
                                        ),
                                        point(
                                            p2.x + px(b.cos() * inner),
                                            p2.y + px(b.sin() * inner),
                                        ),
                                    ),
                                    st,
                                );
                            }
                        }
                        window.paint_path(path, rgb(appearance.color));
                    }

                    // Node bodies, drawn through a level-of-detail ladder. Every
                    // visible node is drawn at every zoom level; only its
                    // representation gets cheaper as it shrinks. A quad is an
                    // instanced primitive whose corner rounding and border are
                    // evaluated analytically in the fragment shader, so a screen
                    // full of node bodies costs one batch rather than one shaped
                    // path each.
                    //
                    // Diamonds are the one shape a quad cannot express, so they
                    // are deferred into one path per color and flushed after the
                    // quad run rather than interleaved with it, which would split
                    // the quad batch once per node.
                    let mut diamonds: Vec<(u32, Path<Pixels>)> = Vec::new();
                    for (index, position) in positions.iter().enumerate() {
                        if hidden[index] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible[index])
                        {
                            continue;
                        }
                        let center = viewport.world_to_screen(*position);
                        if !bounds.contains(&center) {
                            continue;
                        }
                        let appearance = node_appearances[index];
                        match appearance.shape {
                            NodeShape::Rect {
                                corner_radius_world,
                                border_color,
                                border_width_pixels,
                            } => {
                                let body = Bounds::centered_at(
                                    center,
                                    size(
                                        px((node_sizes[index].width * viewport.zoom()).max(1.0)),
                                        px((node_sizes[index].height * viewport.zoom()).max(1.0)),
                                    ),
                                );
                                profile::count(Counter::Quads, 1);
                                if body.size.height < px(SHELL_LOD_MIN_PIXELS) {
                                    // Sub-pixel corners and borders resolve to
                                    // nothing a viewer can see, so the flat fill
                                    // is both cheaper and visually identical.
                                    window.paint_quad(fill(body, rgb(appearance.color)));
                                } else {
                                    window.paint_quad(quad(
                                        body,
                                        px(corner_radius_world * viewport.zoom()),
                                        rgb(appearance.color),
                                        px(border_width_pixels),
                                        rgb(border_color),
                                        BorderStyle::default(),
                                    ));
                                }
                            }
                            NodeShape::Square => {
                                profile::count(Counter::Quads, 1);
                                let diameter = px(appearance.radius_pixels * 2.0);
                                window.paint_quad(fill(
                                    Bounds::centered_at(center, size(diameter, diameter)),
                                    rgb(appearance.color),
                                ));
                            }
                            NodeShape::Diamond => {
                                let r = px(appearance.radius_pixels);
                                let path = match diamonds
                                    .iter_mut()
                                    .find(|(color, _)| *color == appearance.color)
                                {
                                    Some((_, path)) => path,
                                    None => {
                                        diamonds.push((
                                            appearance.color,
                                            Path::new(point(px(0.0), px(0.0))),
                                        ));
                                        &mut diamonds.last_mut().expect("just pushed").1
                                    }
                                };
                                let a = point(center.x, center.y - r);
                                let b = point(center.x + r, center.y);
                                let c = point(center.x, center.y + r);
                                let d = point(center.x - r, center.y);
                                path.push_triangle((a, b, c), st);
                                path.push_triangle((a, c, d), st);
                            }
                            NodeShape::None => {
                                // A node whose element content was dropped by the
                                // content budget still has to appear. Its own fill
                                // is the only description of it the graph has.
                                if has_content[index] && !content_drawn[index] {
                                    window.paint_quad(fill(
                                        Bounds::centered_at(
                                            center,
                                            size(
                                                px((node_sizes[index].width * viewport.zoom())
                                                    .max(1.0)),
                                                px((node_sizes[index].height * viewport.zoom())
                                                    .max(1.0)),
                                            ),
                                        ),
                                        rgb(appearance.color),
                                    ));
                                }
                            }
                        }
                    }
                    for (color, path) in diamonds {
                        window.paint_path(path, rgb(color));
                        profile::count(Counter::DrawCalls, 1);
                    }
                });

                window.paint_layer(bounds, |window| {
                    let _scope = profile::scope(Phase::PaintOverlays);

                    if show_handles {
                        let handle_size = px(CONNECTION_HANDLE_SIZE_WORLD * viewport.zoom());
                        let outer_half = handle_size * 0.5;
                        let inner_half = (outer_half - px(1.0)).max(px(0.0));
                        let mut handle_borders = Path::new(point(px(0.0), px(0.0)));
                        let mut handle_fills = Path::new(point(px(0.0), px(0.0)));
                        macro_rules! push_square {
                            ($path:expr, $center:expr, $half:expr) => {{
                                let center = $center;
                                let half = $half;
                                let a = point(center.x - half, center.y - half);
                                let b = point(center.x + half, center.y - half);
                                let c = point(center.x + half, center.y + half);
                                let d = point(center.x - half, center.y + half);
                                $path.push_triangle((a, b, c), st);
                                $path.push_triangle((a, c, d), st);
                            }};
                        }
                        for (index, position) in positions.iter().enumerate() {
                            if hidden[index] {
                                continue;
                            }
                            if visible_nodes
                                .as_ref()
                                .is_some_and(|visible| !visible[index])
                            {
                                continue;
                            }
                            let center = viewport.world_to_screen(*position);
                            for kind in [HandleKind::Target, HandleKind::Source] {
                                let handle = connection_handle_position(
                                    center,
                                    node_appearances[index].radius_pixels,
                                    kind,
                                    target_handle_position,
                                    source_handle_position,
                                    viewport.zoom(),
                                );
                                push_square!(handle_borders, handle, outer_half);
                                if inner_half > px(0.0) {
                                    push_square!(handle_fills, handle, inner_half);
                                }
                            }
                        }
                        window.paint_path(handle_borders, rgb(0x1e90ff));
                        window.paint_path(handle_fills, rgb(0xffffff));
                    }

                    for &index in selected.iter() {
                        if hidden[index] {
                            continue;
                        }
                        if visible_nodes
                            .as_ref()
                            .is_some_and(|visible| !visible[index])
                        {
                            continue;
                        }
                        let center = viewport.world_to_screen(positions[index]);
                        let selection_size =
                            if matches!(node_appearances[index].shape, NodeShape::None) {
                                size(
                                    px((node_sizes[index].width * viewport.zoom()).max(1.0)),
                                    px((node_sizes[index].height * viewport.zoom()).max(1.0)),
                                )
                            } else {
                                size(px(18.0), px(18.0))
                            };
                        window.paint_quad(outline(
                            Bounds::centered_at(center, selection_size),
                            rgb(style.selection_color),
                            BorderStyle::default(),
                        ));
                        if !show_handles {
                            let handle_size = px(CONNECTION_HANDLE_SIZE_WORLD * viewport.zoom());
                            for kind in [HandleKind::Target, HandleKind::Source] {
                                let handle = connection_handle_position(
                                    center,
                                    node_appearances[index].radius_pixels,
                                    kind,
                                    target_handle_position,
                                    source_handle_position,
                                    viewport.zoom(),
                                );
                                window.paint_quad(fill(
                                    Bounds::centered_at(handle, size(handle_size, handle_size)),
                                    rgb(0xffffff),
                                ));
                                window.paint_quad(outline(
                                    Bounds::centered_at(handle, size(handle_size, handle_size)),
                                    rgb(0x1e90ff),
                                    BorderStyle::default(),
                                ));
                            }
                        }
                        if show_resize_handles && show_resize_controls[index] {
                            let resize_color =
                                resize_control_colors[index].unwrap_or(style.selection_color);
                            for direction in resize_directions[index]
                                .as_deref()
                                .unwrap_or(&RESIZE_DIRECTIONS)
                                .iter()
                                .copied()
                            {
                                let resize = resize_handle_position(
                                    center,
                                    node_sizes[index],
                                    viewport.zoom(),
                                    direction,
                                );
                                let handle_size = if matches!(
                                    direction,
                                    crate::ResizeDirection::NorthWest
                                        | crate::ResizeDirection::NorthEast
                                        | crate::ResizeDirection::SouthEast
                                        | crate::ResizeDirection::SouthWest
                                ) {
                                    px(8.0)
                                } else {
                                    px(7.0)
                                };
                                let corner = matches!(
                                    direction,
                                    crate::ResizeDirection::NorthWest
                                        | crate::ResizeDirection::NorthEast
                                        | crate::ResizeDirection::SouthEast
                                        | crate::ResizeDirection::SouthWest
                                );
                                window.paint_quad(fill(
                                    Bounds::centered_at(resize, size(handle_size, handle_size)),
                                    rgb(if corner { resize_color } else { 0xffffff }),
                                ));
                                window.paint_quad(outline(
                                    Bounds::centered_at(resize, size(handle_size, handle_size)),
                                    rgb(resize_color),
                                    BorderStyle::default(),
                                ));
                            }
                        }
                    }
                    if show_resize_handles {
                        for (index, always_visible) in
                            resize_controls_always_visible.iter().copied().enumerate()
                        {
                            if !always_visible
                                || selected.contains(&index)
                                || hidden[index]
                                || !show_resize_controls[index]
                                || visible_nodes
                                    .as_ref()
                                    .is_some_and(|visible| !visible[index])
                            {
                                continue;
                            }
                            let center = viewport.world_to_screen(positions[index]);
                            let resize_color =
                                resize_control_colors[index].unwrap_or(style.selection_color);
                            for direction in resize_directions[index]
                                .as_deref()
                                .unwrap_or(&RESIZE_DIRECTIONS)
                                .iter()
                                .copied()
                            {
                                let resize = resize_handle_position(
                                    center,
                                    node_sizes[index],
                                    viewport.zoom(),
                                    direction,
                                );
                                let corner = matches!(
                                    direction,
                                    crate::ResizeDirection::NorthWest
                                        | crate::ResizeDirection::NorthEast
                                        | crate::ResizeDirection::SouthEast
                                        | crate::ResizeDirection::SouthWest
                                );
                                let handle_size = if corner { px(8.0) } else { px(7.0) };
                                window.paint_quad(fill(
                                    Bounds::centered_at(resize, size(handle_size, handle_size)),
                                    rgb(if corner { resize_color } else { 0xffffff }),
                                ));
                                window.paint_quad(outline(
                                    Bounds::centered_at(resize, size(handle_size, handle_size)),
                                    rgb(resize_color),
                                    BorderStyle::default(),
                                ));
                            }
                        }
                    }
                    if let Some(selection) = multi_selection {
                        let top_left = viewport.world_to_screen(selection.origin);
                        let bottom_right = viewport.world_to_screen(WorldPoint::new(
                            selection.origin.x + selection.size.width,
                            selection.origin.y + selection.size.height,
                        ));
                        let padding = px(MULTI_SELECTION_PADDING_PIXELS);
                        let rectangle = Bounds::new(
                            point(top_left.x - padding, top_left.y - padding),
                            size(
                                bottom_right.x - top_left.x + padding * 2.0,
                                bottom_right.y - top_left.y + padding * 2.0,
                            ),
                        );
                        window
                            .paint_quad(fill(rectangle, rgba((style.selection_color << 8) | 0x18)));
                        window.paint_quad(outline(
                            rectangle,
                            rgba((style.selection_color << 8) | 0xc0),
                            BorderStyle::default(),
                        ));
                    }
                    if let Some((start, end)) = marquee {
                        let origin = point(start.x.min(end.x), start.y.min(end.y));
                        let rectangle = Bounds::new(
                            origin,
                            size((start.x - end.x).abs(), (start.y - end.y).abs()),
                        );
                        window.paint_quad(outline(
                            rectangle,
                            rgba(0x1e90ffb0),
                            BorderStyle::default(),
                        ));
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let simulation = canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _, window, cx| {
                let (playing, zooming, auto_panning) =
                    cx.read_entity(&graph_handle, |graph: &Graph, _| {
                        (
                            graph.playing,
                            graph.smooth_zoom.is_some(),
                            graph.node_drag_auto_pan_delta() != ViewportPoint::new(0.0, 0.0),
                        )
                    });
                if playing || zooming || auto_panning {
                    window.request_animation_frame();
                    cx.update_entity(&graph_handle, |graph, cx| {
                        if graph.playing {
                            graph.step_layout();
                        }
                        graph.advance_smooth_zoom();
                        graph.advance_node_drag_auto_pan();
                        cx.notify();
                    });
                }
            },
        )
        .absolute()
        .size_full();

        let playing = self.playing;
        let edge_detail_stride = edge_stride;
        let controls = div()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .bg(rgb(0xf7f7f7))
            .border(px(1.0))
            .border_color(rgb(0xcccccc))
            .rounded(px(6.0))
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap_2()
            .cursor_default()
            .child(format!(
                "nodes: {}  edges: {}",
                self.model.nodes.len(),
                self.model.edges.len()
            ))
            .child(if edge_detail_stride > 1 {
                format!(
                    "edge LOD 1/{edge_detail_stride}  ({:.1} ms avg)",
                    self.governor.average_ms()
                )
            } else {
                "full edge detail".to_string()
            })
            .child(format!("layout frame: {}", self.sim_tick))
            .child(self.announcement.clone());

        let play_button = div()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .size(px(28.0))
            .rounded_full()
            .bg(if playing {
                rgb(0x4CAF50)
            } else {
                rgb(0xeeeeee)
            })
            .border(px(1.0))
            .border_color(rgb(0xcccccc))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|graph, _: &MouseDownEvent, window, cx| {
                    if graph.playing {
                        graph.stop_layout();
                        let size = graph
                            .canvas_size()
                            .unwrap_or_else(|| window.viewport_size());
                        graph.fit_to_view(size, px(40.0));
                    } else {
                        graph.start_layout()
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );

        let canvas_cursor = if self.pan_drag_position.is_some() || self.drag_nodes.is_some() {
            CursorStyle::ClosedHand
        } else if self.pointer_over_handle {
            CursorStyle::Crosshair
        } else if self.pointer_over_graph_item {
            CursorStyle::Arrow
        } else {
            CursorStyle::OpenHand
        };
        // Most graphs label no edges at all; walking the edge list every frame
        // to discover that is a pass worth not taking.
        let labelled_edges: &[Edge] = if self.scene.any_edge_labels {
            &self.model.edges
        } else {
            &[]
        };
        let edge_labels = labelled_edges
            .iter()
            .filter(|edge| reconnecting_edge != Some(edge.id))
            .filter_map(|edge| {
                let label = edge.label.as_ref()?;
                let source = self
                    .model
                    .store
                    .node_center_absolute(self.model.node(edge.source)?);
                let target = self
                    .model
                    .store
                    .node_center_absolute(self.model.node(edge.target)?);
                let midpoint = self.viewport.world_to_screen(WorldPoint::new(
                    (source.x + target.x) * 0.5,
                    (source.y + target.y) * 0.5,
                ));
                Some(
                    div()
                        .absolute()
                        .left(midpoint.x)
                        .top(midpoint.y)
                        .px_1()
                        .bg(rgba(0xffffffe0))
                        .child(label.clone()),
                )
            })
            .collect::<Vec<_>>();

        div()
            .id("gpug-graph")
            .track_focus(&self.focus)
            .key_context("gpug-graph")
            .size_full()
            .bg(rgb(self.renderer.style().background))
            .cursor(canvas_cursor)
            .child(simulation)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseDownEvent, window, cx| {
                    graph.focus.focus(window);
                    if let Some((index, direction)) =
                        graph.resize_at_screen_position(event.position)
                    {
                        let node = &graph.model.nodes[index];
                        let mut control = crate::NodeResizeControl::new(node.id, direction);
                        control.begin(
                            graph.screen_to_world(event.position),
                            graph.model.store.runtimes[&node.id].bounds(),
                        );
                        graph.resize_node = Some((index, control));
                        graph.announce("Resizing node");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    // A handle is the more specific target when its hit box
                    // overlaps a selected edge endpoint. This keeps a drag
                    // from the node's source handle anchored to that node.
                    if let Some((key, _)) = graph
                        .handle_at_screen_position(event.position, false)
                        .or_else(|| graph.handle_at_screen_position(event.position, true))
                    {
                        let pointer = graph.screen_to_world(event.position);
                        graph.connection.arm(key.clone(), ConnectionIntent::Create);
                        graph.connection.begin(pointer);
                        graph.events.push(GraphEvent::ConnectStart {
                            from: key,
                            intent: ConnectionIntent::Create,
                        });
                        graph.model.store.dirty.mark_connection();
                        graph
                            .gestures
                            .claim(GestureOwner::Handle, Gesture::Connection);
                        graph.announce("Connection started");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    if let Some((key, intent)) = graph.reconnect_at_screen_position(event.position)
                    {
                        let pointer = graph.screen_to_world(event.position);
                        graph.connection.arm(key.clone(), intent.clone());
                        graph.connection.begin(pointer);
                        graph
                            .events
                            .push(GraphEvent::ConnectStart { from: key, intent });
                        graph.model.store.dirty.mark_connection();
                        graph
                            .gestures
                            .claim(GestureOwner::Handle, Gesture::Connection);
                        graph.announce("Reconnecting edge");
                        graph.flush();
                        cx.notify();
                        return;
                    }
                    let hit = graph.node_at_screen_position(event.position);
                    if let Some(index) = hit {
                        let id = graph.model.nodes[index].id;
                        graph
                            .model
                            .select_node_for_pointer(id, event.modifiers.shift);
                        if !graph.model.store.node_selected(&graph.model.nodes[index]) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                        if !graph.node_allows_drag_at_screen_position(
                            &graph.model.nodes[index],
                            event.position,
                        ) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                        graph.begin_selected_node_drag(event.position, id);
                    } else if !event.modifiers.shift
                        && graph.begin_multi_selection_drag(event.position)
                    {
                        graph.announce("Dragging selection");
                    } else if let Some(index) = graph.edge_index_at_screen_position(event.position)
                    {
                        let id = graph.model.edges[index].id;
                        graph
                            .model
                            .select_edge(id, event.modifiers.shift, event.modifiers.shift);
                    } else {
                        graph.smooth_zoom = None;
                        if event.modifiers.shift {
                            graph.selection_start = Some(event.position);
                            graph.selection_current = Some(event.position);
                            graph.gestures.claim(
                                GestureOwner::Marquee,
                                Gesture::Marquee {
                                    start: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                    current: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                },
                            );
                        } else {
                            graph.model.clear_selection();
                            graph.pan_drag_position = Some(event.position);
                            graph.pointer = Some(PointerController::begin(
                                ViewportPoint::new(
                                    event.position.x / px(1.0),
                                    event.position.y / px(1.0),
                                ),
                                graph.gestures.drag_threshold,
                                Vec::new(),
                            ));
                            graph.gestures.claim(
                                GestureOwner::Viewport,
                                Gesture::ViewportPan {
                                    previous: ViewportPoint::new(
                                        event.position.x / px(1.0),
                                        event.position.y / px(1.0),
                                    ),
                                },
                            );
                        }
                    }
                    graph.flush();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|graph, event: &KeyDownEvent, _, cx| {
                if graph.handle_key(event) {
                    cx.stop_propagation();
                    cx.notify();
                }
                graph.flush();
            }))
            .on_mouse_move(cx.listener(|graph, event: &MouseMoveEvent, _, cx| {
                if let Some((index, control)) = graph.resize_node {
                    graph.pointer_over_handle = false;
                    let pointer = graph.screen_to_world(event.position);
                    if let Some(resized) = control.update(pointer) {
                        let id = graph.model.nodes[index].id;
                        graph.model.resize_node_from_bounds(id, resized, true);
                    }
                } else if matches!(graph.connection.state, ConnectionState::Connecting { .. }) {
                    let pointer = graph.screen_to_world(event.position);
                    let target_is_end = matches!(
                        graph.connection.state,
                        ConnectionState::Connecting {
                            ref from,
                            ..
                        } if from.kind == HandleKind::Source
                    );
                    let target = graph
                        .handle_at_screen_position(event.position, target_is_end)
                        .map(|(key, center)| graph.connection_handle(key, center));
                    graph.pointer_over_handle = target.is_some();
                    graph
                        .connection
                        .update(pointer, target.as_ref(), std::iter::empty());
                    graph.model.store.dirty.mark_connection();
                } else if let Some(items) = graph.drag_nodes.clone() {
                    graph.pointer_over_handle = false;
                    if let Some(pointer) = &mut graph.pointer {
                        if !pointer.update(ViewportPoint::new(
                            event.position.x / px(1.0),
                            event.position.y / px(1.0),
                        )) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                    }
                    let world = graph.screen_to_world(event.position);
                    let targets = items
                        .into_iter()
                        .map(|(index, offset)| {
                            (
                                graph.model.nodes[index].id,
                                WorldPoint::new(world.x - offset.x, world.y - offset.y),
                            )
                        })
                        .collect::<Vec<_>>();
                    graph.model.move_nodes(&targets, true);
                    graph.layout_initialized = false;
                } else if let Some(start) = graph.selection_start {
                    graph.pointer_over_handle = false;
                    graph.selection_current = Some(event.position);
                    let a = graph.screen_to_world(start);
                    let b = graph.screen_to_world(event.position);
                    let rect = WorldBounds::new(
                        WorldPoint::new(a.x.min(b.x), a.y.min(b.y)),
                        crate::WorldSize::new((a.x - b.x).abs(), (a.y - b.y).abs()),
                    );
                    graph
                        .model
                        .select_rect(rect, graph.selection_mode, event.modifiers.shift);
                } else if let Some(previous) = graph.pan_drag_position {
                    if let Some(pointer) = &mut graph.pointer {
                        if !pointer.update(ViewportPoint::new(
                            event.position.x / px(1.0),
                            event.position.y / px(1.0),
                        )) {
                            graph.flush();
                            cx.notify();
                            return;
                        }
                    }
                    graph.pan_by(point(
                        event.position.x - previous.x,
                        event.position.y - previous.y,
                    ));
                    graph.pan_drag_position = Some(event.position);
                    graph.pointer_over_graph_item = false;
                    graph.pointer_over_handle = false;
                } else {
                    graph.pointer_over_handle = graph.is_handle_at_screen_position(event.position);
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                }
                graph.flush();
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|graph, event: &MouseUpEvent, _, cx| {
                    let owner = graph.gestures.owner();
                    if let Some((index, mut control)) = graph.resize_node.take() {
                        let node = graph.model.nodes[index].clone();
                        let pointer = graph.screen_to_world(event.position);
                        let bounds = control
                            .end(pointer)
                            .unwrap_or_else(|| graph.model.store.runtimes[&node.id].bounds());
                        graph.model.resize_node_from_bounds(node.id, bounds, false);
                        graph.announce("Node resize finished");
                    }
                    let pending = graph.connection.pending();
                    let mut connected = false;
                    if let Some((connection, intent)) = graph.connection.finish() {
                        graph.model.store.dirty.mark_connection();
                        if matches!(intent, ConnectionIntent::Create) {
                            let mut edge =
                                Edge::new(connection.source.node, connection.target.node)
                                    .with_id(graph.next_edge_id);
                            edge.source_handle = connection.source.id.as_deref().map(str::to_owned);
                            edge.target_handle = connection.target.id.as_deref().map(str::to_owned);
                            if graph.model.add_edge_with_id(edge.clone()) {
                                graph.next_edge_id = graph.next_edge_id.wrapping_add(1);
                                graph.events.push(GraphEvent::Connected(edge.clone()));
                                graph.announce("Edge connected");
                                connected = true;
                            }
                        } else {
                            let id = match intent {
                                ConnectionIntent::ReconnectSource(id)
                                | ConnectionIntent::ReconnectTarget(id) => id,
                                ConnectionIntent::Create => unreachable!(),
                            };
                            if graph.model.reconnect(id, intent, &connection) {
                                let edge = graph
                                    .model
                                    .edge(id)
                                    .expect("reconnected edge remains in the model")
                                    .clone();
                                graph.events.push(GraphEvent::Reconnected {
                                    id,
                                    edge: edge.clone(),
                                });
                                graph.announce("Edge reconnected");
                                connected = true;
                            }
                        }
                    } else {
                        graph.connection.cancel();
                        graph.model.store.dirty.mark_connection();
                    }
                    if let Some((from, intent, _)) = pending {
                        // The controller only keeps the pointer it was last
                        // moved to; the release position is the truthful drop
                        // point, and the two differ when the button comes up
                        // without an intervening move.
                        graph.events.push(GraphEvent::ConnectEnd {
                            from,
                            intent,
                            position: graph.screen_to_world(event.position),
                            connected,
                        });
                    }
                    if let Some(items) = graph.drag_nodes.take() {
                        let targets = items
                            .iter()
                            .map(|(index, _)| {
                                let node = &graph.model.nodes[*index];
                                (node.id, graph.model.store.node_position_absolute(node))
                            })
                            .collect::<Vec<_>>();
                        graph.model.move_nodes(&targets, false);
                        graph.announce("Node drag finished");
                    }
                    graph.auto_pan_edge_since = None;
                    graph.pan_drag_position = None;
                    if let Some(pointer) = &mut graph.pointer {
                        pointer.end();
                    }
                    graph.pointer = None;
                    graph.selection_start = None;
                    graph.selection_current = None;
                    graph.gestures.finish();
                    if owner == Some(GestureOwner::Marquee) {
                        graph.announce("Marquee selection finished");
                    }
                    if owner == Some(GestureOwner::Viewport) {
                        graph
                            .events
                            .push(GraphEvent::ViewportChanged(graph.viewport));
                    }
                    graph.pointer_over_graph_item =
                        graph.graph_item_at_screen_position(event.position);
                    graph.pointer_over_handle = graph.is_handle_at_screen_position(event.position);
                    graph.flush();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|graph, event: &ScrollWheelEvent, _, cx| {
                let dy = event.delta.pixel_delta(px(16.0)).y;
                if dy != px(0.0) {
                    let steps = ((dy / px(16.0)).abs()).max(0.01);
                    // A gentle per-notch scale keeps mouse wheels smooth while
                    // still preserving proportional high-resolution trackpad input.
                    let factor = 1.04_f32.powf(steps);
                    let factor = if dy > px(0.0) { factor } else { factor.recip() };
                    graph.queue_smooth_zoom(factor, event.position);
                    cx.notify();
                }
            }))
            .child(graph_canvas)
            .child(node_content_layer)
            .children(uncached_node_contents)
            .children(edge_labels)
            .child(controls)
            .child(play_button)
    }
}

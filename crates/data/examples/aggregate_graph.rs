//! Live worker-prepared aggregate graph. Wheel zoom, pan, select, then drill down.
use gpug::{Graph, GraphEvent, GraphRenderer, GraphStyle, NodeAppearance, NodeShape, Viewport};
use gpug_data::{gpug_bridge::*, spatial::*, worker::*, *};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, App, AppContext, Application, Context, Entity,
    MouseButton, Render, Subscription, Window, WindowOptions,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
#[path = "aggregate_graph/replay.rs"]
mod replay;

#[derive(Clone)]
struct Request {
    index: Arc<SpatialIndex>,
    bounds: WorldRect,
    zoom: f64,
    max_zoom: f64,
    focus: Option<NodeId>,
}
struct Prepared {
    scene: LodGraphData,
    started: Instant,
    finished: Instant,
}
struct AggregateView {
    graph: Entity<Graph>,
    index: Arc<SpatialIndex>,
    alternate: Arc<SpatialIndex>,
    revision: u64,
    worker: QueryWorker<Request, Result<Prepared, String>>,
    client: Option<ClientId>,
    pending: Option<Request>,
    expected: Option<Ticket>,
    area: (f64, f64),
    camera: Viewport,
    polling: bool,
    scene: Option<LodGraphData>,
    selected_node: Option<gpug::NodeId>,
    selected_source: Option<NodeId>,
    selected_group: Option<usize>,
    context_page: usize,
    inspecting: bool,
    inspection: String,
    status: String,
    replay: Option<replay::Replay>,
    paint_ready: bool,
    _observe: Subscription,
}
impl AggregateView {
    fn new(
        graph: Entity<Graph>,
        index: Arc<SpatialIndex>,
        alternate: Arc<SpatialIndex>,
        cx: &mut Context<Self>,
    ) -> Self {
        let camera = graph.read(cx).viewport();
        let mut lod = LodWorkspace::new();
        let mut cache = ConnectionCache::new();
        let worker = QueryWorker::spawn(1, move |_, request: Request, token| {
            let started = Instant::now();
            if token.is_cancelled() {
                return Err("superseded".into());
            }
            let plan = Arc::new(
                lod.prepare_focused(
                    request.index,
                    request.bounds,
                    LodOptions {
                        zoom: request.zoom,
                        max_zoom: request.max_zoom,
                        aggregate_below_pixels: 100.,
                        max_items: 128,
                        ..Default::default()
                    },
                    request.focus,
                )
                .map_err(|error| error.to_string())?,
            );
            let connections = cache
                .prepare(plan, ConnectionBudget::default(), || token.is_cancelled())
                .map_err(|error| error.to_string())?;
            let scene =
                to_lod_graph_data(connections, "aggregate").map_err(|error| error.to_string())?;
            Ok(Prepared {
                scene,
                started,
                finished: Instant::now(),
            })
        })
        .expect("start aggregate worker");
        let observe = cx.observe(&graph, |view, graph, cx| {
            let events = graph.update(cx, |graph, _| graph.take_events());
            let camera = graph.read(cx).viewport();
            if camera != view.camera {
                view.camera = camera;
                view.queue(cx);
            }
            for event in events {
                if let GraphEvent::SelectionChanged { nodes, edges } = event {
                    view.selected_node = nodes.last().copied();
                    view.selected_source = view.scene.as_ref().and_then(|scene| {
                        let item = view.selected_node.and_then(|id| scene.node_item(id))?;
                        scene.connections().plan().members(item)?.next()
                    });
                    view.selected_group = view
                        .scene
                        .as_ref()
                        .and_then(|scene| edges.last().and_then(|id| scene.edge_group(*id)));
                    view.inspection.clear();
                    if view.selected_group.is_some() {
                        view.inspect(cx);
                    }
                    cx.notify();
                }
            }
        });
        Self {
            graph,
            index,
            alternate,
            revision: 1,
            worker,
            client: None,
            pending: None,
            expected: None,
            area: (0., 0.),
            camera,
            polling: false,
            scene: None,
            selected_node: None,
            selected_source: None,
            selected_group: None,
            context_page: 0,
            inspecting: false,
            inspection: String::new(),
            status: "Preparing view…".into(),
            replay: std::env::args()
                .any(|arg| arg == "--replay")
                .then(replay::Replay::new),
            paint_ready: false,
            _observe: observe,
        }
    }
    fn queue(&mut self, cx: &mut Context<Self>) {
        self.paint_ready = false;
        if self.area.0 <= 0. || self.area.1 <= 0. {
            return;
        }
        let zoom = f64::from(self.camera.zoom());
        self.pending = Some(Request {
            index: Arc::clone(&self.index),
            bounds: WorldRect {
                x: -f64::from(self.camera.pan().x / px(1.)) / zoom,
                y: -f64::from(self.camera.pan().y / px(1.)) / zoom,
                width: self.area.0 / zoom,
                height: self.area.1 / zoom,
            },
            zoom,
            max_zoom: f64::from(self.camera.max_zoom()),
            focus: self.selected_source,
        });
        self.expected = None;
        self.status = "Updating representations…".into();
        cx.notify();
        if self.polling {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if !matches!(this.update(cx, |view, cx| view.poll(cx)), Ok(true)) {
                break;
            }
        })
        .detach();
    }
    fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        let camera = self.graph.read(cx).viewport();
        if camera != self.camera {
            self.camera = camera;
            self.queue(cx);
        }
        if self.client.is_none() {
            match self.worker.register() {
                Ok(client) => self.client = Some(client),
                Err(WorkerError::Busy) => return true,
                Err(error) => return self.fail(error.to_string(), cx),
            }
        }
        let client = self.client.expect("registered");
        if let Some(input) = self.pending.take() {
            match self.worker.submit(
                client,
                WorkStamp {
                    dataset: 1,
                    revision: self.revision,
                },
                input,
            ) {
                Ok(ticket) => self.expected = Some(ticket),
                Err(error) if error.error == WorkerError::Busy => {
                    self.pending = Some(error.input);
                    return true;
                }
                Err(error) => return self.fail(error.error.to_string(), cx),
            }
        }
        match self.worker.try_take(client) {
            Ok(Some(done)) if Some(done.ticket) == self.expected => {
                let prepared = match done.outcome {
                    Ok(Ok(prepared)) => prepared,
                    Ok(Err(error)) => return self.fail(error, cx),
                    Err(error) => return self.fail(error.to_string(), cx),
                };
                let install_started = Instant::now();
                let mut scene = prepared.scene;
                let items = scene.data.nodes.len();
                let groups = scene.connections().groups().len();
                let selected = self
                    .selected_source
                    .and_then(|id| scene.connections().node_item(id))
                    .and_then(|item| scene.data.nodes.get(item).map(|node| node.id));
                let data = std::mem::take(&mut scene.data);
                let result = self.graph.update(cx, |graph, cx| {
                    graph.take_events();
                    let result = graph.set_data(data);
                    if result.is_ok() {
                        if let Some(id) = selected {
                            graph.select_node(id);
                        }
                        graph.take_events();
                    }
                    cx.notify();
                    result
                });
                if let Err(error) = result {
                    return self.fail(error.to_string(), cx);
                }
                self.scene = Some(scene);
                if let Some(replay) = &mut self.replay {
                    replay.stages = Some((
                        prepared.started,
                        prepared.finished,
                        install_started,
                        Instant::now(),
                    ));
                }
                self.paint_ready = true;
                self.expected = None;
                self.polling = false;
                self.selected_node = selected;
                self.selected_group = None;
                self.inspection.clear();
                self.status = format!(
                    "Revision {} · {items} items · {groups} groups",
                    self.revision
                );
                cx.notify();
                false
            }
            Ok(_) | Err(WorkerError::Busy) => true,
            Err(error) => self.fail(error.to_string(), cx),
        }
    }
    fn fail(&mut self, error: String, cx: &mut Context<Self>) -> bool {
        self.status = error;
        self.polling = false;
        self.expected = None;
        self.pending = None;
        cx.notify();
        false
    }
    fn switch_source(&mut self, cx: &mut Context<Self>) {
        let Some(revision) = self.revision.checked_add(1) else {
            self.fail("Source revision exhausted".into(), cx);
            return;
        };
        std::mem::swap(&mut self.index, &mut self.alternate);
        self.revision = revision;
        // Invalidate publication before queueing, even if pane size is zero.
        self.expected = None;
        self.pending = None;
        self.selected_source = self.selected_source.filter(|id| {
            self.index
                .snapshot()
                .node_index(*id)
                .is_some_and(|dense| !self.index.snapshot().nodes()[dense as usize].hidden)
        });
        self.selected_group = None;
        self.inspection.clear();
        self.context_page = 0;
        self.queue(cx);
    }
    fn focus_source(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_source {
            let source = self.index.snapshot();
            let Some(dense) = source.node_index(id) else {
                return;
            };
            let b = source.nodes()[dense as usize].bounds;
            let zoom = self.camera.max_zoom();
            let pan = point(
                px((self.area.0 * 0.5 - (b.x + b.width * 0.5) * f64::from(zoom)) as f32),
                px((self.area.1 * 0.5 - (b.y + b.height * 0.5) * f64::from(zoom)) as f32),
            );
            self.camera = Viewport::new(pan, zoom).with_max_zoom(zoom);
            self.graph.update(cx, |graph, cx| {
                graph.sync_viewport(self.camera);
                cx.notify();
            });
            self.queue(cx);
        }
    }
    fn inspect(&mut self, cx: &mut Context<Self>) {
        if self.inspecting {
            return;
        }
        let Some(group) = self.selected_group else {
            return;
        };
        let Some(scene) = &self.scene else {
            return;
        };
        let connections = scene.connections().clone();
        let plan = Arc::clone(connections.plan());
        self.inspecting = true;
        let task = cx.background_executor().spawn(async move {
            connections
                .members(group)
                .map(|ids| {
                    ids.take(12)
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let ids = task.await;
            this.update(cx, |view, cx| {
                view.inspecting = false;
                let current = view
                    .scene
                    .as_ref()
                    .is_some_and(|scene| Arc::ptr_eq(scene.connections().plan(), &plan));
                if current && view.selected_group == Some(group) {
                    view.inspection = format!("First source edge IDs: {ids}");
                    cx.notify();
                } else if view.selected_group.is_some() {
                    view.inspect(cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
impl Render for AggregateView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let paint_ack = canvas(
            |_, _, _| (),
            move |_, _, _, cx| {
                weak.update(cx, |view, cx| view.replay_painted(cx)).ok();
            },
        )
        .absolute()
        .size(px(1.));
        let mut members =
            String::from("Select a node or aggregate to inspect its source membership.");
        let mut context = Vec::new();
        let mut context_count = 0;
        const PAGE_SIZE: usize = 6;
        let mut hidden = 0;
        if let Some(scene) = &self.scene {
            hidden = scene.connections().hidden_edges;
            if let Some(item) = self.selected_node.and_then(|id| scene.node_item(id)) {
                let plan = scene.connections().plan();
                let ids = plan
                    .members(item)
                    .unwrap()
                    .take(12)
                    .map(|id| id.0.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                members = format!(
                    "{} source members. First IDs: {ids}",
                    plan.items()[item].represented_nodes
                );
            }
            context_count = scene.context_groups().len();
            self.context_page = self
                .context_page
                .min(context_count.saturating_sub(1) / PAGE_SIZE);
            context = scene
                .context_groups()
                .iter()
                .skip(self.context_page * PAGE_SIZE)
                .take(PAGE_SIZE)
                .map(|index| {
                    let group = &scene.connections().groups()[*index];
                    let kind = if matches!(
                        (group.source, group.target),
                        (PlanEndpoint::Item(_), PlanEndpoint::Item(_))
                    ) {
                        "internal"
                    } else {
                        "beyond displayed nodes"
                    };
                    (*index, format!("{} {kind} edges", group.edge_count))
                })
                .collect();
        }
        let page_count = context_count.div_ceil(PAGE_SIZE).max(1);
        let all_context_edges: usize = self.scene.as_ref().map_or(0, |scene| {
            scene
                .context_groups()
                .iter()
                .map(|i| scene.connections().groups()[*i].edge_count)
                .sum()
        });
        let this = cx.entity().downgrade();
        let measure = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, _, cx| {
                this.update(cx, |view, cx| {
                    let size = (
                        f64::from(bounds.size.width / px(1.)),
                        f64::from(bounds.size.height / px(1.)),
                    );
                    if view.area != size {
                        view.area = size;
                        view.queue(cx);
                    }
                })
                .ok();
            },
        )
        .absolute()
        .size_full();
        div().relative().size_full().flex().flex_col().bg(rgb(0xf3f6fa))
            .child(div().h(px(48.)).flex_none().flex().items_center().gap_3().px_3()
                .child(self.status.clone())
                .child(div().id("focus-source").cursor_pointer().px_3().py_2().bg(rgb(0xc8e6ef)).child("Focus first source node")
                    .on_mouse_down(MouseButton::Left, cx.listener(|view, _, _, cx| { view.focus_source(cx); cx.stop_propagation(); })))
                .child(div().id("switch-source").cursor_pointer().px_3().py_2().bg(rgb(0xdbe6cf)).child("Switch source")
                    .on_mouse_down(MouseButton::Left, cx.listener(|view, _, _, cx| { view.switch_source(cx); cx.stop_propagation(); }))))
            .child(div().flex_1().min_h_0().flex()
                .child(div().relative().flex_1().min_w_0().overflow_hidden().child(measure).child(self.graph.clone()))
                .child(div().w(px(240.)).flex_none().p_3().flex().flex_col().gap_3().overflow_hidden()
                    .child("Source context").child(members)
                    .child(format!("{all_context_edges} edges in {context_count} context groups; {hidden} hidden-incident edges."))
                    .child("Groups summarize internal and outside connections.")
                    .child(div().flex().gap_3()
                        .child(div().id("context-previous").cursor_pointer().child("Previous")
                            .on_mouse_down(MouseButton::Left, cx.listener(|view, _, _, cx| {
                                view.context_page = view.context_page.saturating_sub(1); cx.notify(); cx.stop_propagation();
                            })))
                        .child(format!("{}/{}", self.context_page + 1, page_count))
                        .child(div().id("context-next").cursor_pointer().child("Next")
                            .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, _, cx| {
                                view.context_page = (view.context_page + 1).min(page_count - 1); cx.notify(); cx.stop_propagation();
                            }))))
                    .children(context.into_iter().map(|(index, label)| div().id(("context", index)).cursor_pointer().p_2().bg(rgb(0xe2e8f0)).child(label)
                        .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, _, cx| {
                            view.selected_group = Some(index); view.inspection.clear(); view.inspect(cx); cx.notify(); cx.stop_propagation();
                        }))))
                    .child(self.inspection.clone())))
            .child(paint_ack)
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let overlap = std::env::args().any(|argument| argument == "--overlap");
    let count = std::env::var("GPUG_DATA_NODES")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(1024);
    if !(1..=1_000_000).contains(&count) {
        return Err("GPUG_DATA_NODES must be 1..=1000000".into());
    }
    let source = Arc::new(GraphSnapshot::new(
        (0..count)
            .map(|id| {
                let mut node = Node::new(
                    id,
                    WorldRect {
                        x: if overlap { 0. } else { (id % 32) as f64 * 220. },
                        y: if overlap { 0. } else { (id / 32) as f64 * 140. },
                        width: 180.,
                        height: 100.,
                    },
                );
                node.renderer = "flow".into();
                node.label = format!("Step {id}");
                node
            })
            .collect(),
        (0..count)
            .flat_map(|id| {
                let mut vertical = Edge::new(id * 2 + 1, id, (id + 32) % count);
                if (id + 1) % count == vertical.target.0 {
                    vertical.source_handle = Some("vertical".into());
                }
                [Edge::new(id * 2, id, (id + 1) % count), vertical]
            })
            .collect(),
    )?);
    // Build both immutable generations before opening the view. Real hosts build
    // replacement indexes on a data worker, then publish the completed Arc.
    let mut alternate_nodes = source.nodes().to_vec();
    for node in &mut alternate_nodes {
        node.label = format!("Updated step {}", node.id.0);
    }
    let alternate = Arc::new(SpatialIndex::new(Arc::new(GraphSnapshot::new(
        alternate_nodes,
        source
            .edges()
            .iter()
            .filter(|edge| edge.id.0 % 2 == 0)
            .cloned()
            .collect(),
    )?)));
    let index = Arc::new(SpatialIndex::new(source));
    Application::new().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                app_id: Some("GPUG Aggregate Graph".into()),
                ..Default::default()
            },
            move |_, cx| {
                let mut renderer = GraphRenderer::new(GraphStyle {
                    content_budget: 128,
                    interactive_edge_budget: 10000,
                    edge_lod_min_pixels: 0.,
                    ..Default::default()
                });
                for (name, color) in [("aggregate", 0xd6b86c), ("flow", 0x27465b)] {
                    renderer.register_node_type(
                        name,
                        move |_: &gpug::Node, _: f32, _: &GraphStyle| NodeAppearance {
                            color,
                            radius_pixels: 4.,
                            shape: NodeShape::Rect {
                                corner_radius_world: 5.,
                                border_color: 0x68808d,
                                border_width_pixels: 1.,
                            },
                        },
                    );
                }
                renderer.register_node_content("aggregate", |node: &gpug::Node, _: f32| {
                    div()
                        .size_full()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(rgb(0x332a17))
                        .child(node.metadata[AGGREGATE_COUNT_KEY].clone())
                        .child("nodes")
                        .into_any_element()
                });
                let counters: Arc<Vec<AtomicUsize>> =
                    Arc::new((0..count).map(|_| AtomicUsize::new(0)).collect());
                renderer.register_node_content("flow", move |node: &gpug::Node, zoom: f32| {
                    let id = node.id.0 as usize;
                    let count = counters[id].load(Ordering::Relaxed);
                    let counters = Arc::clone(&counters);
                    div()
                        .size_full()
                        .p(px(8. * zoom))
                        .bg(rgb(0x27465b))
                        .text_size(px(13. * zoom))
                        .text_color(rgb(0xffffff))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(node.metadata[LABEL_KEY].clone())
                        .child(
                            div()
                                .id(("run", node.id.0))
                                .cursor_pointer()
                                .bg(rgb(0x376d78))
                                .p_2()
                                .child(format!("Run · {count}"))
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    counters[id].fetch_add(1, Ordering::Relaxed);
                                    window.refresh();
                                    cx.stop_propagation();
                                }),
                        )
                        .into_any_element()
                });
                let graph = cx.new(|cx| {
                    Graph::builder()
                        .data(gpug::GraphData::default())
                        .renderer(renderer)
                        .interactive_layout(false)
                        .viewport(
                            Viewport::new(
                                if overlap {
                                    point(px(300.), px(200.))
                                } else {
                                    point(px(20.), px(20.))
                                },
                                0.08,
                            )
                            .with_max_zoom(1.5),
                        )
                        .build(cx)
                        .expect("empty valid graph")
                });
                cx.new(|cx| AggregateView::new(graph, index, alternate, cx))
            },
        )
        .expect("open aggregate view");
    });
    Ok(())
}

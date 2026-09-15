use super::*;
use gpug::analysis::worker::{
    ClientId, GraphQueryTask, QueryWorker, Ticket, WorkStamp, WorkerError,
};
use gpui::{Context, Entity, Render, Window};
use std::time::Duration;

pub struct DataFlowView {
    graph: Entity<Graph>,
    source: Arc<GraphSnapshot>,
    worker: QueryWorker<GraphQueryTask, Result<gpug::GraphData, String>>,
    client: Option<ClientId>,
    pending: Option<GraphQueryTask>,
    expected: Option<Ticket>,
    stamp: WorkStamp,
    polling: bool,
    status: String,
}

impl DataFlowView {
    pub fn new(graph: Entity<Graph>, source: Arc<GraphSnapshot>, cx: &mut Context<Self>) -> Self {
        let mut workspace = QueryWorkspace::new();
        let worker = QueryWorker::spawn(1, move |_, task: GraphQueryTask, token| {
            let result = workspace
                .execute_cancellable(&task.graph, &task.query, task.budget, || {
                    token.is_cancelled()
                })
                .map_err(|error| error.to_string())?;
            if token.is_cancelled() {
                return Err("superseded".into());
            }
            if result.truncated {
                return Err("query budget exhausted; previous view retained".into());
            }
            let selection = task
                .graph
                .induced_subgraph(result.nodes)
                .map_err(|error| error.to_string())?;
            if token.is_cancelled() {
                return Err("superseded".into());
            }
            let mut data =
                gpug_bridge::to_graph_data(&selection).map_err(|error| error.to_string())?;
            decorate(&mut data);
            Ok(data)
        })
        .expect("start demonstration query worker");
        let mut view = Self {
            graph,
            source,
            worker,
            client: None,
            pending: None,
            expected: None,
            stamp: WorkStamp {
                dataset: 1,
                revision: 1,
            },
            polling: false,
            status: "Loading…".into(),
        };
        view.request(5, cx);
        view
    }

    fn request(&mut self, hops: usize, cx: &mut Context<Self>) {
        // Invalidate the host's expected result BEFORE attempting a possibly-Busy
        // submit. Otherwise an old completion could win while the newest intent
        // is still waiting to enter the worker queue.
        self.expected = None;
        self.pending = Some(GraphQueryTask {
            graph: Arc::clone(&self.source),
            query: Arc::new(
                Query::parse(&format!("NEIGHBORS 0 OUTGOING HOPS {hops}"))
                    .expect("fixed query grammar"),
            ),
            budget: QueryBudget::default(),
        });
        self.status = format!("Loading {hops}-hop neighborhood…");
        cx.notify();
        if self.polling {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                // Only one polling task exists, only while a request is pending.
                // Busy is retried on a later dispatch; never spin in pointer input.
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let active = this.update(cx, |view, cx| view.poll(cx));
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        if self.client.is_none() {
            match self.worker.register() {
                Ok(client) => self.client = Some(client),
                Err(WorkerError::Busy) => return true,
                Err(error) => return self.fail(error.to_string(), cx),
            }
        }
        let client = self.client.expect("registered above");
        if let Some(task) = self.pending.take() {
            match self.worker.submit(client, self.stamp, task) {
                Ok(ticket) => self.expected = Some(ticket),
                Err(error) if error.error == WorkerError::Busy => {
                    self.pending = Some(error.input);
                    return true;
                }
                Err(error) => return self.fail(error.error.to_string(), cx),
            }
        }
        match self.worker.try_take(client) {
            Ok(Some(completion)) => {
                if self.expected != Some(completion.ticket) || completion.ticket.stamp != self.stamp
                {
                    return true;
                }
                self.expected = None;
                let data = match completion.outcome {
                    Ok(Ok(data)) => data,
                    Ok(Err(error)) => return self.fail(error, cx),
                    Err(error) => return self.fail(error.to_string(), cx),
                };
                let count = data.nodes.len();
                // This installs only the prepared working set. Queries, induced
                // subgraphs and conversion all ran on the data worker.
                let result = self.graph.update(cx, |graph, cx| {
                    let result = graph.set_data(data);
                    if result.is_ok() {
                        cx.notify();
                    }
                    result
                });
                if let Err(error) = result {
                    return self.fail(error.to_string(), cx);
                }
                self.status = format!("{count} source nodes · drag headers, click Run step");
                self.polling = false;
                cx.notify();
                false
            }
            Ok(None) | Err(WorkerError::Busy) => true,
            Err(error) => self.fail(error.to_string(), cx),
        }
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) -> bool {
        self.status = message;
        self.pending = None;
        self.expected = None;
        self.polling = false;
        cx.notify();
        false
    }
}

impl Render for DataFlowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .bg(rgb(0x172335))
                    .text_color(rgb(0xffffff))
                    .children([1usize, 2, 5].map(|hops| {
                        div()
                            .id(("hops", hops))
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x285b69))
                            .child(format!("{hops} hops"))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.request(hops, cx);
                                    cx.stop_propagation();
                                }),
                            )
                    }))
                    .child(self.status.clone()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.graph.clone()),
            )
    }
}

pub fn decorate(data: &mut gpug::GraphData) {
    for node in &mut data.nodes {
        node.nodrag.push(WorldBounds::new(
            WorldPoint::new(0., 70.),
            WorldSize::new(230., 60.),
        ));
    }
}

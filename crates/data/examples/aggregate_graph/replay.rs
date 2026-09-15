//! Opt-in, bounded live CPU-paint replay. This is not a GPU presentation timestamp.
use super::*;

pub(super) struct Replay {
    step: usize,
    started: Instant,
    rows: Vec<String>,
    pub(super) stages: Option<(Instant, Instant, Instant, Instant)>,
}
impl Replay {
    pub(super) fn new() -> Self {
        Self {
            step: 0,
            started: Instant::now(),
            rows: Vec::with_capacity(91),
            stages: None,
        }
    }
    fn event(&self) -> &'static str {
        match self.step {
            0 => "initial",
            1..=30 => "pan",
            31..=60 => "zoom",
            61..=75 => "focus",
            _ => "source",
        }
    }
}
impl AggregateView {
    pub(super) fn replay_painted(&mut self, cx: &mut Context<Self>) {
        if !self.paint_ready {
            return;
        }
        let Some(replay) = &mut self.replay else {
            return;
        };
        self.paint_ready = false;
        let elapsed = replay.started.elapsed().as_micros();
        let scene = self.scene.as_ref().expect("installed scene");
        let connections = scene.connections();
        assert!(Arc::ptr_eq(connections.plan().index(), &self.index));
        assert_eq!(
            connections
                .groups()
                .iter()
                .map(|group| group.edge_count)
                .sum::<usize>()
                + connections.hidden_edges,
            self.index.snapshot().edges().len()
        );
        let (worker_start, worker_end, install_start, install_end) =
            replay.stages.take().expect("installed timing");
        let delay = worker_start.duration_since(replay.started).as_micros();
        let worker = worker_end.duration_since(worker_start).as_micros();
        let delivery = install_start.duration_since(worker_end).as_micros();
        let install = install_end.duration_since(install_start).as_micros();
        let paint = install_end.elapsed().as_micros();
        replay.rows.push(format!(
            "{},{},{elapsed},{delay},{worker},{delivery},{install},{paint},{},{},{}",
            replay.step,
            replay.event(),
            connections.plan().items().len(),
            connections.groups().len(),
            self.revision
        ));
        if replay.step == 90 {
            let replay = self.replay.take().unwrap();
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .spawn(async move {
                        println!("step,event,event_to_cpu_paint_us,request_delay_us,worker_us,delivery_us,install_us,paint_us,items,groups,revision");
                        for row in replay.rows {
                            println!("{row}");
                        }
                    })
                    .await;
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
            return;
        }
        replay.step += 1;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            this.update(cx, |view, cx| view.replay_advance(cx)).ok();
        })
        .detach();
    }
    fn replay_advance(&mut self, cx: &mut Context<Self>) {
        let replay = self.replay.as_mut().unwrap();
        replay.started = Instant::now();
        let step = replay.step;
        match step {
            1..=60 => {
                let zoom = if step <= 30 {
                    self.camera.zoom()
                } else if step.is_multiple_of(2) {
                    1.5
                } else {
                    0.08
                };
                self.camera = Viewport::new(
                    point(
                        self.camera.pan().x + px(if step.is_multiple_of(2) { 20. } else { -20. }),
                        self.camera.pan().y,
                    ),
                    zoom,
                )
                .with_max_zoom(1.5);
                self.graph.update(cx, |graph, cx| {
                    graph.sync_viewport(self.camera);
                    cx.notify();
                });
                self.queue(cx);
            }
            61..=75 => {
                self.selected_source = Some(NodeId(
                    (self.index.snapshot().nodes().len() as u64 / 2 + step as u64 * 13)
                        % self.index.snapshot().nodes().len() as u64,
                ));
                self.focus_source(cx);
            }
            _ => self.switch_source(cx),
        }
    }
}

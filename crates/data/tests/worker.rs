use gpug_data::worker::*;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn retry<T>(mut action: impl FnMut() -> Result<T, WorkerError>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match action() {
            Ok(v) => return v,
            Err(WorkerError::Busy) if Instant::now() < deadline => std::thread::yield_now(),
            Err(e) => panic!("{e}"),
        }
    }
}
fn submit<I, O>(
    worker: &QueryWorker<I, O>,
    client: ClientId,
    stamp: WorkStamp,
    mut input: I,
) -> Ticket {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match worker.submit(client, stamp, input) {
            Ok(ticket) => return ticket,
            Err(error) if error.error == WorkerError::Busy && Instant::now() < deadline => {
                input = error.input;
                std::thread::yield_now();
            }
            Err(error) => panic!("{}", error.error),
        }
    }
}
fn take<I, O>(worker: &QueryWorker<I, O>, client: ClientId) -> Completion<O> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = retry(|| worker.try_take(client)) {
            return result;
        }
        assert!(Instant::now() < deadline, "completion did not arrive");
        std::thread::yield_now();
    }
}
fn received<T>(receiver: &Receiver<T>) -> T {
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("controlled worker event")
}

#[test]
fn bursts_coalesce_per_client_and_round_robin_is_fair() {
    let (start, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (events, ran) = mpsc::channel();
    let worker = QueryWorker::spawn(2, move |_, input: usize, cancellation| {
        if input == 0 {
            start.send(()).unwrap();
            received(&gate);
        }
        events.send((input, cancellation.is_cancelled())).unwrap();
        input * 2
    })
    .unwrap();
    let a = retry(|| worker.register());
    let b = retry(|| worker.register());
    assert_eq!(failure(|| worker.register()), WorkerError::ClientCapacity);
    let initial = submit(
        &worker,
        a,
        WorkStamp {
            dataset: 5,
            revision: 0,
        },
        0,
    );
    received(&started);
    let mut latest = initial;
    for i in 1..=100 {
        latest = submit(
            &worker,
            a,
            WorkStamp {
                dataset: 5,
                revision: i as u64,
            },
            i,
        );
    }
    let other = submit(
        &worker,
        b,
        WorkStamp {
            dataset: 6,
            revision: 1,
        },
        1000,
    );
    let stats = retry(|| worker.stats());
    assert_eq!(stats.pending, 2);
    assert_eq!(stats.coalesced, 99);
    assert_eq!(stats.in_flight, Some(initial));
    assert!(!retry(|| worker.is_current(initial)));
    release.send(()).unwrap();
    assert_eq!(received(&ran), (0, true));
    assert_eq!(received(&ran), (1000, false));
    assert_eq!(received(&ran), (100, false));
    let result = take(&worker, a);
    assert_eq!(result.ticket, latest);
    assert_eq!(result.outcome.unwrap(), 200);
    let result = take(&worker, b);
    assert_eq!(result.ticket, other);
    assert_eq!(result.outcome.unwrap(), 2000);
    let stats = retry(|| worker.stats());
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.ready, 0);
    assert_eq!(stats.discarded, 1);
    assert_eq!(stats.completed, 2);
    worker.shutdown_and_join().unwrap();
}

#[test]
fn reused_client_slots_cannot_accept_an_old_completion() {
    let (start, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let worker = QueryWorker::spawn(1, move |_, input: u64, _| {
        if input == 1 {
            start.send(()).unwrap();
            received(&gate);
        }
        input
    })
    .unwrap();
    let old = retry(|| worker.register());
    submit(&worker, old, WorkStamp::default(), 1);
    received(&started);
    retry(|| worker.unregister(old));
    let new = retry(|| worker.register());
    assert_ne!(old, new);
    let failed = worker.submit(old, WorkStamp::default(), 88).unwrap_err();
    assert_eq!(failed.error, WorkerError::UnknownClient);
    assert_eq!(failed.input, 88);
    let ticket = submit(
        &worker,
        new,
        WorkStamp {
            dataset: 2,
            revision: 0,
        },
        2,
    );
    release.send(()).unwrap();
    let result = take(&worker, new);
    assert_eq!(result.ticket, ticket);
    assert_eq!(result.outcome.unwrap(), 2);
    assert_eq!(failure(|| worker.try_take(old)), WorkerError::UnknownClient);
    worker.shutdown_and_join().unwrap();
}

#[test]
fn ready_results_are_invalidated_by_cancellation_and_new_requests() {
    let (notify, notifications) = mpsc::channel();
    let worker = QueryWorker::spawn_with_notify(
        1,
        |_, value: u64, _| value,
        move || {
            notify.send(()).unwrap();
        },
    )
    .unwrap();
    let client = retry(|| worker.register());
    let old = submit(&worker, client, WorkStamp::default(), 1);
    received(&notifications);
    retry(|| worker.cancel(client));
    assert!(!retry(|| worker.is_current(old)));
    assert!(retry(|| worker.try_take(client)).is_none());
    submit(&worker, client, WorkStamp::default(), 2);
    received(&notifications);
    let latest = submit(
        &worker,
        client,
        WorkStamp {
            dataset: 1,
            revision: 1,
        },
        3,
    );
    received(&notifications);
    let result = take(&worker, client);
    assert_eq!(result.ticket, latest);
    assert_eq!(result.outcome.unwrap(), 3);
    worker.shutdown_and_join().unwrap();
}

#[test]
fn shutdown_is_nonblocking_and_ignores_noncooperative_results() {
    let (start, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (observed, observation) = mpsc::channel();
    let worker = QueryWorker::spawn(1, move |_, _: (), token| {
        start.send(()).unwrap();
        received(&gate);
        observed.send(token.is_cancelled()).unwrap();
        7
    })
    .unwrap();
    let client = retry(|| worker.register());
    submit(&worker, client, WorkStamp::default(), ());
    received(&started);
    worker.shutdown(); // Must return while the callback is still blocked on gate.
    assert!(worker.is_stopped());
    assert_eq!(
        worker
            .submit(client, WorkStamp::default(), ())
            .unwrap_err()
            .error,
        WorkerError::Closed
    );
    release.send(()).unwrap();
    assert!(received(&observation));
    assert_eq!(
        retry(|| match worker.try_take(client) {
            Err(WorkerError::Closed) => Ok(()),
            Err(e) => Err(e),
            Ok(_) => panic!("shutdown exposed a result"),
        }),
        ()
    );
    worker.shutdown_and_join().unwrap();
}

#[test]
fn panicking_handlers_report_failure_and_stop_the_worker() {
    let worker: QueryWorker<(), ()> =
        QueryWorker::spawn(1, |_, _, _| panic!("deliberate query failure")).unwrap();
    let client = retry(|| worker.register());
    let ticket = submit(&worker, client, WorkStamp::default(), ());
    let result = take(&worker, client);
    assert_eq!(result.ticket, ticket);
    assert_eq!(result.outcome, Err(WorkerError::Panicked));
    worker.shutdown_and_join().unwrap();
}

#[test]
fn idle_shutdown_and_invalid_capacity_are_handled() {
    assert!(QueryWorker::<(), ()>::spawn(0, |_, _, _| ()).is_err());
    for _ in 0..25 {
        QueryWorker::<(), ()>::spawn(1, |_, _, _| ())
            .unwrap()
            .shutdown_and_join()
            .unwrap();
    }
}

fn failure<T>(mut action: impl FnMut() -> Result<T, WorkerError>) -> WorkerError {
    retry(|| match action() {
        Err(WorkerError::Busy) => Err(WorkerError::Busy),
        Err(error) => Ok(error),
        Ok(_) => panic!("expected error"),
    })
}

#[test]
fn built_in_graph_worker_preserves_snapshot_stamp_and_exact_results() {
    use gpug_data::query::*;
    use gpug_data::*;
    use std::sync::Arc;
    let graph = Arc::new(
        GraphSnapshot::new(
            vec![
                Node::new(50, WorldRect::default()),
                Node::new(90, WorldRect::default()),
            ],
            vec![Edge::new(1, 50, 90)],
        )
        .unwrap(),
    );
    let worker = spawn_graph_query_worker(1, || {}).unwrap();
    let client = retry(|| worker.register());
    let stamp = WorkStamp {
        dataset: 9,
        revision: 5,
    };
    let ticket = submit(
        &worker,
        client,
        stamp,
        GraphQueryTask {
            graph,
            query: Arc::new(Query::parse("NEIGHBORS 50 OUTGOING HOPS 1").unwrap()),
            budget: QueryBudget::default(),
        },
    );
    let completion = take(&worker, client);
    assert_eq!(completion.ticket, ticket);
    let result = completion.outcome.unwrap().unwrap();
    assert_eq!(result.nodes, vec![NodeId(50), NodeId(90)]);
    assert_eq!(result.depths, vec![0, 1]);
    assert!(!result.truncated);
    worker.shutdown_and_join().unwrap();
}

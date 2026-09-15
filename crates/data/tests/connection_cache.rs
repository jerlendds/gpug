use gpug_data::{spatial::*, *};
use std::sync::Arc;

fn index() -> Arc<SpatialIndex> {
    Arc::new(SpatialIndex::new(Arc::new(
        GraphSnapshot::new(
            (0..64)
                .map(|id| {
                    Node::new(
                        id,
                        WorldRect {
                            x: id as f64 * 10.,
                            width: 2.,
                            height: 2.,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            (0..63).map(|id| Edge::new(id, id, id + 1)).collect(),
        )
        .unwrap(),
    )))
}
fn plan(index: Arc<SpatialIndex>, x: f64, coarse: bool) -> Arc<OwnedLodPlan> {
    Arc::new(
        LodWorkspace::new()
            .prepare(
                index,
                WorldRect {
                    x,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                },
                LodOptions {
                    zoom: 60.,
                    max_items: if coarse { 1 } else { 512 },
                    ..Default::default()
                },
            )
            .unwrap(),
    )
}

#[test]
fn changed_view_geometry_reuses_buffers_but_binds_the_new_plan() {
    let index = index();
    let first = plan(Arc::clone(&index), 0., true);
    let second = plan(index, 100., true);
    assert_ne!(first.items()[0].bounds, second.items()[0].bounds);
    let mut cache = ConnectionCache::new();
    let a = cache
        .prepare(first, ConnectionBudget::default(), || false)
        .unwrap();
    let b = cache
        .prepare(Arc::clone(&second), ConnectionBudget::default(), || false)
        .unwrap();
    assert_eq!(cache.stats().builds, 1);
    assert_eq!(cache.stats().hits, 1);
    assert!(Arc::ptr_eq(b.plan(), &second));
    assert!(std::ptr::eq(a.groups().as_ptr(), b.groups().as_ptr()));
    assert_eq!(
        a.members(0).unwrap().collect::<Vec<_>>(),
        b.members(0).unwrap().collect::<Vec<_>>()
    );
    cache.clear();
    assert_eq!(a.groups()[0].edge_count, 63);
    cache
        .prepare(second, ConnectionBudget::default(), || false)
        .unwrap();
    assert_eq!(cache.stats().builds, 2);
}

#[test]
fn equal_item_counts_with_different_membership_or_index_identity_miss() {
    let index = index();
    let first = plan(Arc::clone(&index), 0., false);
    let second = plan(Arc::clone(&index), 100., false);
    assert_eq!(first.items().len(), second.items().len());
    assert_ne!(
        first.members(0).unwrap().next(),
        second.members(0).unwrap().next()
    );
    let mut cache = ConnectionCache::new();
    cache
        .prepare(first, ConnectionBudget::default(), || false)
        .unwrap();
    let actual = cache
        .prepare(Arc::clone(&second), ConnectionBudget::default(), || false)
        .unwrap();
    let expected = second
        .connections(ConnectionBudget::default(), || false)
        .unwrap();
    assert_eq!(cache.stats().builds, 2);
    for (group, expected_group) in actual.groups().iter().zip(expected.groups()) {
        assert_eq!(
            (
                group.source,
                group.target,
                group.edge_count,
                group.weight_sum
            ),
            (
                expected_group.source,
                expected_group.target,
                expected_group.edge_count,
                expected_group.weight_sum
            )
        );
    }
    assert_eq!(actual.groups().len(), expected.groups().len());
    // Even an index over the same Arc<GraphSnapshot> has a distinct partition identity.
    let rebuilt = Arc::new(SpatialIndex::new(Arc::clone(index.snapshot())));
    cache
        .prepare(
            plan(rebuilt, 100., false),
            ConnectionBudget::default(),
            || false,
        )
        .unwrap();
    assert_eq!(cache.stats().builds, 3);
    assert_eq!(cache.stats().hits, 0);
}

#[test]
fn cache_hits_honor_budgets_and_cancellation_without_evicting_success() {
    let plan = plan(index(), 0., true);
    let mut cache = ConnectionCache::new();
    let initial = cache
        .prepare(Arc::clone(&plan), ConnectionBudget::default(), || false)
        .unwrap();
    for (budget, expected) in [
        (
            ConnectionBudget {
                max_nodes: 0,
                ..Default::default()
            },
            ConnectionPlanError::NodeLimit,
        ),
        (
            ConnectionBudget {
                max_edges: 0,
                ..Default::default()
            },
            ConnectionPlanError::EdgeLimit,
        ),
        (
            ConnectionBudget {
                max_groups: 0,
                ..Default::default()
            },
            ConnectionPlanError::GroupLimit,
        ),
    ] {
        assert_eq!(
            cache
                .prepare(Arc::clone(&plan), budget, || false)
                .unwrap_err(),
            expected
        );
    }
    let mut checks = 0;
    assert_eq!(
        cache
            .prepare(Arc::clone(&plan), ConnectionBudget::default(), || {
                checks += 1;
                checks == 2
            })
            .unwrap_err(),
        ConnectionPlanError::Cancelled
    );
    let different = self::plan(Arc::clone(plan.index()), 0., false);
    assert_eq!(
        cache
            .prepare(
                different,
                ConnectionBudget {
                    max_groups: 1,
                    ..Default::default()
                },
                || false
            )
            .unwrap_err(),
        ConnectionPlanError::GroupLimit
    );
    let result = cache
        .prepare(plan, ConnectionBudget::default(), || false)
        .unwrap();
    assert_eq!(cache.stats().builds, 1);
    assert_eq!(cache.stats().hits, 1);
    assert!(std::ptr::eq(
        result.groups().as_ptr(),
        initial.groups().as_ptr()
    ));
}

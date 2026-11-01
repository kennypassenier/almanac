//! Tests of setting undeliverable entries aside (T1).

use super::*;

#[tokio::test]
async fn an_entry_that_can_never_be_delivered_is_eventually_set_aside() {
    // T1. Left pending, one unmappable payload is retried forever,
    // drives the worker to half-hourly polling for every other
    // source, and eventually raises a backlog alert about a queue
    // of exactly one dead event.
    let (state, dir) = state_rejecting_everything().await;
    let notifier = Notifier::disabled();
    let mut permanent = PermanentFailures::new();

    for pass in 1..PERMANENT_FAILURES_BEFORE_DEAD {
        drain_once_tracking(&state, &mut permanent, &notifier).await;
        assert_eq!(
            state.journal.pending().unwrap().len(),
            1,
            "still retrying on pass {pass} — one bad classification must not be enough"
        );
    }

    drain_once_tracking(&state, &mut permanent, &notifier).await;

    assert!(
        state.journal.pending().unwrap().is_empty(),
        "after {PERMANENT_FAILURES_BEFORE_DEAD} permanent failures it must stop being retried"
    );

    let dead = state.journal.dead().unwrap();
    assert_eq!(dead.len(), 1, "and must still be readable");
    assert_eq!(dead[0].0.id, "doomed");
    assert!(
        !dead[0].1.is_empty(),
        "with the reason, which is what makes it fixable"
    );
    // The error's text already carries its remedy; the stored reason must not add it a second time.
    let remedy_marks = dead[0].1.matches(" — ").count();
    assert_eq!(remedy_marks, 1, "one remedy, not two: {}", dead[0].1);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn setting_an_entry_aside_is_reported() {
    // The source was told 202. Quietly giving up on that payload
    // would make that a lie nobody ever hears about.
    let (state, dir) = state_rejecting_everything().await;
    let notify = crate::shell::testing::NotifyStub::start().await;
    let notifier = Notifier::to(reqwest::Client::new(), &notify.url);
    let mut permanent = PermanentFailures::new();

    for _ in 0..PERMANENT_FAILURES_BEFORE_DEAD {
        drain_once_tracking(&state, &mut permanent, &notifier).await;
    }

    assert_eq!(notify.ops().await, vec![ops::ENTRY_SET_ASIDE.to_string()]);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_dead_entry_survives_compaction_with_its_reason() {
    // Compaction rewrites the log; dropping the dead entry there
    // would discard a payload the source was told had been
    // accepted, and lose the reason it failed.
    let (state, dir) = state_rejecting_everything().await;
    let notifier = Notifier::disabled();
    let mut permanent = PermanentFailures::new();

    for _ in 0..PERMANENT_FAILURES_BEFORE_DEAD {
        drain_once_tracking(&state, &mut permanent, &notifier).await;
    }
    state.journal.compact().await.unwrap();

    let dead = state.journal.dead().unwrap();
    assert_eq!(dead.len(), 1, "the dead entry must survive compaction");
    assert!(state.journal.pending().unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

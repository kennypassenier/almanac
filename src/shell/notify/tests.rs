use super::*;

#[test]
fn a_notifier_with_no_webhook_is_disabled_rather_than_broken() {
    let notifier = Notifier::disabled();
    assert!(!notifier.is_enabled());
}

#[tokio::test]
async fn sending_without_a_webhook_is_a_no_op_not_a_panic() {
    // The paths that notify are already failure paths; a missing
    // webhook must not turn a reverted update into a crash.
    Notifier::disabled()
        .send(Event {
            op: ops::UPDATE_REVERTED,
            ok: false,
            version: "0.2.0".to_string(),
            error: Some("did not come up".to_string()),
        })
        .await;
}

#[test]
fn the_payload_matches_the_shape_home_assistant_already_parses() {
    // The automation reads op/ok/version/error and defaults the
    // last two. A renamed field would silently produce log lines
    // with no version and no reason.
    let json = serde_json::to_value(Event {
        op: ops::UPDATE_APPLIED,
        ok: true,
        version: "0.2.0".to_string(),
        error: None,
    })
    .unwrap();

    assert_eq!(json["op"], "almanac-update");
    assert_eq!(json["ok"], true);
    assert_eq!(json["version"], "0.2.0");
    assert!(
        json.get("error").is_none(),
        "a successful event must not carry an empty error field — the log line would read \
             'error: null'"
    );
}

#[test]
fn a_failure_carries_its_reason() {
    let json = serde_json::to_value(Event {
        op: ops::UPDATE_UNVERIFIED,
        ok: false,
        version: "0.2.0".to_string(),
        error: Some("signature does not verify".to_string()),
    })
    .unwrap();

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"], "signature does not verify");
}

#[test]
fn the_operation_names_are_distinct_so_alerts_do_not_collapse_into_each_other() {
    // Home Assistant dedupes pushes by op. Two different problems
    // sharing a name would mean the second never reaches the phone.
    let all = [
        ops::UPDATE_APPLIED,
        ops::UPDATE_REVERTED,
        ops::UPDATE_UNVERIFIED,
        ops::JOURNAL_BACKLOG,
    ];
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len());
}

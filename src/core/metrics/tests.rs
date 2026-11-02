use super::*;

#[test]
fn a_fresh_set_of_counters_renders_all_zeroes_rather_than_nothing() {
    // An absent series and a zero series are different to
    // Prometheus: a rate() over a series that only appears on the
    // first delivery has no baseline to compare against.
    let rendered = Metrics::default().render(Some(0), "1.2.3");
    for name in [
        "almanac_events_accepted_total",
        "almanac_events_delivered_total",
        "almanac_deliveries_failed_total",
        "almanac_entries_dead_total",
        "almanac_token_refreshes_total",
    ] {
        assert!(
            rendered.contains(&format!("{name} 0\n")),
            "{name} is missing from a fresh scrape"
        );
    }
}

#[test]
fn every_counter_climbs_independently() {
    let m = Metrics::default();
    m.accepted();
    m.accepted();
    m.delivered(3);
    m.failed(1);
    m.dead(4);
    m.token_refreshed();

    let rendered = m.render(Some(7), "0.1.0");
    assert!(rendered.contains("almanac_events_accepted_total 2\n"));
    assert!(rendered.contains("almanac_events_delivered_total 3\n"));
    assert!(rendered.contains("almanac_deliveries_failed_total 1\n"));
    assert!(rendered.contains("almanac_entries_dead_total 4\n"));
    assert!(rendered.contains("almanac_token_refreshes_total 1\n"));
    assert!(rendered.contains("almanac_journal_pending 7\n"));
}

#[test]
fn an_unreadable_journal_is_not_reported_as_an_empty_one() {
    // The failure this guards against is quiet and expensive: a
    // journal that cannot be read renders as "0 pending", the
    // backlog dashboard goes flat and green, and the alert that
    // should fire never does.
    let rendered = Metrics::default().render(None, "0.1.0");
    assert!(
        !rendered.contains("almanac_journal_pending 0"),
        "an unreadable journal must not render as an empty one"
    );
    assert!(rendered.contains("almanac_journal_readable 0\n"));
}

#[test]
fn a_readable_journal_says_so() {
    let rendered = Metrics::default().render(Some(0), "0.1.0");
    assert!(rendered.contains("almanac_journal_pending 0\n"));
    assert!(rendered.contains("almanac_journal_readable 1\n"));
}

#[test]
fn the_version_series_is_left_to_the_kit() {
    let rendered = Metrics::default().render(Some(0), "0.1.3");
    assert!(
        !rendered.contains("almanac_build_info"),
        "build_info is the kit's series since 3.0.0"
    );
}

#[test]
fn every_series_is_preceded_by_its_help_and_type() {
    // Not decoration: a scrape missing TYPE makes Prometheus guess
    // untyped, and rate() on an untyped series silently does the
    // wrong thing.
    let rendered = Metrics::default().render(Some(0), "0.1.0");
    for line in rendered.lines().filter(|l| !l.starts_with('#')) {
        let name = line.split([' ', '{']).next().unwrap();
        assert!(
            rendered.contains(&format!("# HELP {name} ")),
            "{name} has no HELP line"
        );
        assert!(
            rendered.contains(&format!("# TYPE {name} ")),
            "{name} has no TYPE line"
        );
    }
}

#[test]
fn the_output_carries_no_identifiers_only_numbers() {
    // The M13 acceptance criterion. Deliberately checked against
    // the *shape* of the output rather than a list of forbidden
    // strings: the only text that may appear is metric names, help
    // text and the version, so anything with a value that is not a
    // number would fail this.
    let m = Metrics::default();
    m.accepted();
    let rendered = m.render(Some(1), "0.1.0");
    for line in rendered.lines().filter(|l| !l.starts_with('#')) {
        let (_, value) = line.rsplit_once(' ').expect("every series has a value");
        assert!(
            value.parse::<f64>().is_ok(),
            "a series carries something that is not a number: {line}"
        );
    }
    // And no label anywhere: the one labelled series, build_info, is
    // the kit's since 3.0.0.
    assert_eq!(rendered.matches('{').count(), 0);
}

#[test]
fn history_write_failures_are_counted_and_remember_the_first_time() {
    let m = Metrics::default();
    assert!(
        m.render(Some(0), "x")
            .contains("almanac_history_write_failures_total 0\n")
    );
    m.history_write_failed(1_000);
    m.history_write_failed(2_000);
    assert!(
        m.render(Some(0), "x")
            .contains("almanac_history_write_failures_total 2\n")
    );
    let c = m.snapshot();
    assert_eq!(
        (c.history_write_failures, c.history_failing_since),
        (2, 1_000)
    );
}

#[test]
fn the_history_failure_line_is_absent_without_failures_and_has_a_remedy_with_them() {
    assert_eq!(history_failure_line(&Counts::default()), None);
    let m = Metrics::default();
    m.history_write_failed(1_700_000_000);
    let line = history_failure_line(&m.snapshot()).expect("a line after a failure");
    assert!(
        line.contains("1 time(s) since 2023-11-14 22:13 UTC"),
        "{line}"
    );
    assert!(line.contains("disk space") && line.contains("permissions"));
}

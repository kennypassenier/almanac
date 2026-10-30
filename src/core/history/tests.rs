use super::*;

fn rec(at: &str, entry: &str, result: DeliveryResult) -> DeliveryRecord {
    DeliveryRecord {
        at: at.to_string(),
        entry_id: entry.to_string(),
        source: "s".to_string(),
        calendar_id: "c".to_string(),
        title: "t".to_string(),
        result,
        reason: None,
        upsert_key: Some("s:k".to_string()),
    }
}

#[test]
fn the_day_is_the_brussels_day() {
    assert_eq!(
        day_key("2026-10-18T22:30:00+00:00").as_deref(),
        Some("2026-10-19")
    );
}

#[test]
fn a_record_survives_the_trip_through_a_row() {
    let original = rec("2026-10-19T08:16:00+00:00", "e1", DeliveryResult::Updated);
    let row = Row::from_record(&original).unwrap();
    assert_eq!(row.day, "2026-10-19");
    assert_eq!(row.to_record(), original);
}

#[test]
fn an_unreadable_timestamp_is_an_error_not_a_dropped_row() {
    let book = Book {
        version: FORMAT_VERSION,
        days: vec![DayRecord {
            date: "2026-10-19".to_string(),
            accepted: 1,
            delivered: 1,
            failed: 0,
            deliveries: vec![rec("yesterday", "e1", DeliveryResult::Created)],
        }],
    };
    assert!(rows_from_book(&book).unwrap_err().contains("e1"));
}

#[test]
fn rows_from_a_book_keep_every_delivery_in_order() {
    let book = Book {
        version: FORMAT_VERSION,
        days: vec![
            DayRecord {
                date: "2026-10-18".to_string(),
                accepted: 1,
                delivered: 1,
                failed: 0,
                deliveries: vec![rec(
                    "2026-10-18T10:00:00+00:00",
                    "a",
                    DeliveryResult::Created,
                )],
            },
            DayRecord {
                date: "2026-10-19".to_string(),
                accepted: 2,
                delivered: 1,
                failed: 1,
                deliveries: vec![
                    rec("2026-10-19T10:00:00+00:00", "b", DeliveryResult::Failed),
                    rec("2026-10-19T11:00:00+00:00", "c", DeliveryResult::Updated),
                ],
            },
        ],
    };
    let ids: Vec<_> = rows_from_book(&book)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn the_counters_split_failures_from_deliveries() {
    let count = count_day(
        "2026-10-19",
        [
            (DeliveryResult::Created, 1),
            (DeliveryResult::Updated, 1),
            (DeliveryResult::Failed, 1),
        ],
    );
    assert_eq!((count.accepted, count.delivered, count.failed), (3, 2, 1));
}

#[test]
fn results_round_trip_through_their_word() {
    for r in [
        DeliveryResult::Created,
        DeliveryResult::Updated,
        DeliveryResult::Failed,
    ] {
        assert_eq!(DeliveryResult::parse(r.as_str()), Some(r));
    }
    assert_eq!(DeliveryResult::parse("other"), None);
}

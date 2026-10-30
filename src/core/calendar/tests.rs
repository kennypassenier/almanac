use super::*;

#[test]
fn absent_optional_fields_are_omitted_from_the_serialized_json() {
    let event = GoogleEvent {
        id: None,
        summary: "Wasmachine klaar".to_string(),
        description: None,
        location: None,
        color_id: None,
        start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
        end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties: None,
    };

    let json = serde_json::to_value(&event).unwrap();
    let obj = json.as_object().unwrap();
    assert!(!obj.contains_key("id"));
    assert!(!obj.contains_key("description"));
    assert!(!obj.contains_key("location"));
    assert!(!obj.contains_key("colorId"));
    assert!(!obj.contains_key("extendedProperties"));
    assert_eq!(obj.get("summary").unwrap(), "Wasmachine klaar");
}

#[test]
fn extended_properties_round_trip_through_json() {
    let mut private = HashMap::new();
    private.insert(
        "almanac_source_id".to_string(),
        "home-assistant:switch.wasmachine".to_string(),
    );

    let event = GoogleEvent {
        id: Some("evt123".to_string()),
        summary: "test".to_string(),
        description: None,
        location: None,
        color_id: Some("2".to_string()),
        start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
        end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties: Some(ExtendedProperties { private }),
    };

    let json = serde_json::to_string(&event).unwrap();
    let round_tripped: GoogleEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped, event);
}

#[test]
fn event_list_response_treats_a_missing_items_key_as_empty() {
    let parsed: EventListResponse = serde_json::from_str("{}").unwrap();
    assert!(parsed.items.is_none());
}

#[test]
fn a_timed_boundary_serializes_the_way_google_expects_it() {
    // This is not decoration. `untagged` on the enum makes the
    // outer `rename_all` apply to variant *names*, so without a
    // rename on the variant itself the fields go out as
    // `date_time` and `time_zone`. Google rejects that, and the
    // symptom is every delivery failing at once with nothing in the
    // code looking wrong. It happened on the first build of K14.
    let json = serde_json::to_value(EventDateTime::timed(
        "2026-09-01T09:00:00+00:00",
        "Europe/Brussels",
    ))
    .unwrap();
    assert_eq!(json["dateTime"], "2026-09-01T09:00:00+00:00");
    assert_eq!(json["timeZone"], "Europe/Brussels");
    assert!(
        json.get("date").is_none(),
        "a timed boundary must not carry a date"
    );
}

#[test]
fn an_all_day_boundary_carries_a_date_and_nothing_else() {
    // Google accepts exactly one of the two. Sending both is a
    // rejected request; sending neither is a rejected request.
    let json = serde_json::to_value(EventDateTime::all_day("2026-09-01")).unwrap();
    assert_eq!(json["date"], "2026-09-01");
    assert!(json.get("dateTime").is_none());
    assert!(json.get("timeZone").is_none());
    assert_eq!(json.as_object().unwrap().len(), 1);
}

#[test]
fn both_boundary_shapes_survive_a_round_trip_through_google_json() {
    // Reading events back is how K2 finds the one to update, so a
    // boundary that serializes correctly but cannot be parsed back
    // would break the upsert rather than the create.
    for original in [
        EventDateTime::timed("2026-09-01T09:00:00+00:00", "UTC"),
        EventDateTime::all_day("2026-09-01"),
    ] {
        let json = serde_json::to_string(&original).unwrap();
        let back: EventDateTime = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original, "round trip changed {json}");
    }
}

#[test]
fn reminders_serialize_as_google_names_them() {
    let json = serde_json::to_value(Reminders {
        use_default: false,
        overrides: vec![ReminderOverride {
            method: "popup".to_string(),
            minutes: 30,
        }],
    })
    .unwrap();
    assert_eq!(json["useDefault"], false);
    assert_eq!(json["overrides"][0]["method"], "popup");
    assert_eq!(json["overrides"][0]["minutes"], 30);
}

#[test]
fn deliberate_silence_omits_the_override_list_rather_than_sending_an_empty_one() {
    // `useDefault: false` with no overrides is how Google is told
    // "no reminders at all", and it is a different instruction from
    // omitting the block, which inherits the calendar's default.
    let json = serde_json::to_value(Reminders {
        use_default: false,
        overrides: vec![],
    })
    .unwrap();
    assert_eq!(json["useDefault"], false);
    assert!(json.get("overrides").is_none());
}

#[test]
fn the_optional_event_fields_are_omitted_when_unset_rather_than_sent_as_null() {
    // A null where Google expects a string is a rejected request,
    // and every profile written before K16/K17 leaves these unset.
    let event = GoogleEvent {
        id: None,
        summary: "s".to_string(),
        description: None,
        location: None,
        color_id: None,
        start: EventDateTime::all_day("2026-09-01"),
        end: EventDateTime::all_day("2026-09-02"),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties: None,
    };
    let json = serde_json::to_value(&event).unwrap();
    for absent in ["transparency", "status", "reminders", "location", "colorId"] {
        assert!(
            json.get(absent).is_none(),
            "{absent} should be omitted, not null"
        );
    }
}

//! What the Try a source page says about a dry run: the verdict against
//! the event already in the calendar, and the path of the call as the
//! steps of the page's trace. Pure — the lookup of the existing event
//! and the mapping itself stay in `shell` and `core::mapping`.

use serde::Serialize;

use crate::core::calendar::GoogleEvent;

/// What really sending the call would do to the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing carries the marker yet: a new event.
    Create,
    /// An event carries the marker and differs from the mapped one.
    Update,
    /// An event carries the marker and is already exactly the mapped one.
    NoChange,
}

/// Compares the mapped event with the one found under its marker. Google's
/// own id is not part of what a source says, so it is left out of the comparison.
pub fn verdict(existing: Option<&GoogleEvent>, mapped: &GoogleEvent) -> Verdict {
    match existing {
        None => Verdict::Create,
        Some(found) => {
            let mut found = found.clone();
            found.id = None;
            let mut mapped = mapped.clone();
            mapped.id = None;
            if found == mapped {
                Verdict::NoChange
            } else {
                Verdict::Update
            }
        }
    }
}

/// One step of the path of a call, in the shape the page's trace takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hop {
    pub title: &'static str,
    pub note: String,
    /// `allowed`, `refused`, or `unknown` for a step the call never reached.
    pub verdict: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

/// How far a call got.
pub enum Outcome<'a> {
    /// No profile carries the source id.
    UnknownSource { source_id: &'a str },
    /// The profile exists and the event step refused the call.
    Refused { rule: String, note: String },
    /// The call maps; `verdict` is `None` when the calendar could not be looked in.
    Mapped {
        summary: &'a str,
        all_day: bool,
        verdict: Option<Verdict>,
    },
}

fn hop(title: &'static str, note: &str, verdict: &'static str, rule: Option<String>) -> Hop {
    Hop {
        title,
        note: note.to_string(),
        verdict,
        rule,
    }
}

fn unreached(title: &'static str, note: &str) -> Hop {
    hop(title, note, "unknown", None)
}

/// The four steps of a call: the call, the source's profile, the event, the calendar.
pub fn hops(source_id: &str, calendar_name: &str, outcome: &Outcome<'_>) -> Vec<Hop> {
    let call = hop("Call", "from the dashboard session", "allowed", None);
    let event_note = "checks every field of the call";
    let calendar_note = "where the event would go";
    match outcome {
        Outcome::UnknownSource { source_id } => vec![
            call,
            hop(
                "Profile",
                &format!("no profile for {source_id}"),
                "refused",
                Some("known source".to_string()),
            ),
            unreached("Event", event_note),
            unreached("Calendar", calendar_note),
        ],
        Outcome::Refused { rule, note } => vec![
            call,
            hop(
                "Profile",
                &format!("{source_id} writes to {calendar_name}"),
                "allowed",
                None,
            ),
            hop("Event", note, "refused", Some(rule.clone())),
            unreached("Calendar", calendar_note),
        ],
        Outcome::Mapped {
            summary,
            all_day,
            verdict,
        } => {
            let kind = if *all_day { "all day" } else { "timed" };
            let calendar = match verdict {
                Some(Verdict::Create) => format!("{calendar_name}: would be created, nothing was"),
                Some(Verdict::Update) => format!("{calendar_name}: would be updated, nothing was"),
                Some(Verdict::NoChange) => format!("{calendar_name}: already up to date"),
                None => format!("{calendar_name}: could not be looked in"),
            };
            vec![
                call,
                hop(
                    "Profile",
                    &format!("{source_id} writes to {calendar_name}"),
                    "allowed",
                    None,
                ),
                hop("Event", &format!("{summary}, {kind}"), "allowed", None),
                hop(
                    "Calendar",
                    &calendar,
                    if verdict.is_some() {
                        "allowed"
                    } else {
                        "unknown"
                    },
                    None,
                ),
            ]
        }
    }
}

/// The rule that refused a payload, in a few words for the trace's chip.
/// Classified from the wording of the mapping's own refusals; anything new
/// falls back to a general label rather than a wrong one.
pub fn rule_of(message: &str) -> &'static str {
    const RULES: [(&str, &str); 14] = [
        ("unknown field", "no unknown fields"),
        ("missing field", "title and start are required"),
        ("invalid type", "field types"),
        ("\"title\" is empty", "title is not empty"),
        ("\"external_id\" is empty", "external_id is not empty"),
        ("IANA", "known time zone"),
        ("at or before", "end is after start"),
        ("\"end\"", "end is RFC3339"),
        ("\"start\"", "start is a date or timestamp"),
        ("duration_", "length above zero"),
        ("Google colour", "Google colour"),
        ("Google event status", "Google status"),
        ("\"reminders\"", "reminder limits"),
        ("Idempotency-Key", "external_id or Idempotency-Key"),
    ];
    RULES
        .iter()
        .find(|(needle, _)| message.contains(needle))
        .map_or("event rules", |(_, rule)| rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::calendar::EventDateTime;

    fn event(summary: &str) -> GoogleEvent {
        GoogleEvent {
            id: None,
            summary: summary.to_string(),
            description: None,
            location: None,
            color_id: None,
            start: EventDateTime::timed("2026-08-28T09:00:00+00:00", "UTC"),
            end: EventDateTime::timed("2026-08-28T10:00:00+00:00", "UTC"),
            transparency: None,
            status: None,
            reminders: None,
            extended_properties: None,
        }
    }

    #[test]
    fn nothing_found_is_a_create() {
        assert_eq!(verdict(None, &event("a")), Verdict::Create);
    }

    #[test]
    fn googles_id_does_not_count_as_a_change() {
        let mut found = event("a");
        found.id = Some("abc".to_string());
        assert_eq!(verdict(Some(&found), &event("a")), Verdict::NoChange);
        assert_eq!(verdict(Some(&found), &event("b")), Verdict::Update);
    }

    #[test]
    fn a_refusal_is_named_by_its_rule() {
        assert_eq!(rule_of("unknown field `allDay`"), "no unknown fields");
        assert_eq!(rule_of("something new"), "event rules");
    }
}

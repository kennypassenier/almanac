//! `GET /history/data/week`: what Almanac wrote in one week, read back
//! from the managed calendars for the week agenda.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Datelike, Days, NaiveDate, SecondsFormat, TimeZone};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::calendar::{EventDateTime, GoogleEvent};
use crate::core::history::{DeliveryRecord, ZONE};
use crate::shell::delivery::UPSERT_PROPERTY;
use crate::shell::history_store;
use crate::shell::state::AppState;

use super::{calendar_names, human, today};

/// Week colours the package offers (`--chart-1` to `--chart-5`).
const SERIES: usize = 5;

/// A week's length, in days.
const WEEK_DAYS: u64 = 7;

#[derive(Deserialize)]
pub(super) struct WeekQuery {
    start: Option<String>,
}

/// Midnight of a day in Brussels, as RFC 3339.
fn midnight(day: NaiveDate) -> String {
    ZONE.from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// The start of an event as the week agenda wants it, and as a person reads it.
fn event_start(start: &EventDateTime) -> Option<(String, String)> {
    match start {
        EventDateTime::Timed { date_time, .. } => {
            let at = DateTime::parse_from_rfc3339(date_time).ok()?;
            Some((date_time.clone(), human(&at.to_rfc3339())))
        }
        EventDateTime::AllDay { date } => {
            let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
            Some((
                midnight(day),
                format!("{} (all day)", day.format("%d/%m/%Y")),
            ))
        }
    }
}

/// What Almanac wrote in the seven days from `start` (a Monday by default
/// this week), read from the managed calendars; only events that carry
/// Almanac's own property count.
pub(super) async fn week(
    State(state): State<Arc<AppState>>,
    Query(q): Query<WeekQuery>,
) -> Result<Json<Value>, chassis::Error> {
    let first = week_start(&state, q.start.as_deref())?;
    let names = calendar_names(&state).await;
    let managed = managed_calendars(&state, names.map());
    let listed = list_week(&state, &managed, first).await?;
    // Only this week's keys are looked up, so the page never reads the
    // whole history for the "delivered" lines.
    let keys: Vec<String> = listed
        .iter()
        .flat_map(|(_, found)| found.iter())
        .filter_map(|e| upsert_key_of(e).cloned())
        .collect();
    let read = history_store::latest_by_key(&state, keys).await?;
    let latest = read.rows;

    let name_of = |id: &str| names.name_of(id);
    let mut events = Vec::new();
    let mut used = BTreeMap::new();
    for (index, (id, found)) in listed.iter().enumerate() {
        let series = index % SERIES + 1;
        for event in found {
            if let Some(json) = event_json(event, id, &name_of(id), series, &latest) {
                used.insert(series, name_of(id));
                events.push(json);
            }
        }
    }
    Ok(Json(json!({
        "start": first.format("%Y-%m-%d").to_string(),
        "unreadable": read.unreadable,
        "events": events,
        "calendars": used.into_iter()
            .map(|(series, name)| json!({ "series": series, "name": name }))
            .collect::<Vec<_>>(),
    })))
}

/// The asked-for day, or the Monday of this week.
fn week_start(state: &AppState, asked: Option<&str>) -> Result<NaiveDate, chassis::Error> {
    match asked {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
            chassis::Error::invalid("that is not a day", "use the week buttons to move")
        }),
        None => {
            let now = NaiveDate::parse_from_str(&today(state), "%Y-%m-%d").unwrap_or_default();
            Ok(now - Days::new(u64::from(now.weekday().num_days_from_monday())))
        }
    }
}

/// The calendars a source writes to, each once, in a colour order that is
/// stable across weeks: by name, then id.
fn managed_calendars(state: &AppState, names: &HashMap<String, String>) -> Vec<String> {
    let mut managed: Vec<String> = state
        .profiles()
        .values()
        .map(|p| p.target_calendar_id.clone())
        .collect();
    managed.sort();
    managed.dedup();
    managed.sort_by_key(|id| (names.get(id).cloned().unwrap_or_default(), id.clone()));
    managed
}

/// Every event in the week, per calendar, in `managed` order.
async fn list_week(
    state: &AppState,
    managed: &[String],
    first: NaiveDate,
) -> Result<Vec<(String, Vec<GoogleEvent>)>, chassis::Error> {
    let (time_min, time_max) = (midnight(first), midnight(first + Days::new(WEEK_DAYS)));
    let mut listed = Vec::new();
    for id in managed {
        let found = state
            .client
            .list_events_between(id, &time_min, &time_max)
            .await
            .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
        listed.push((id.clone(), found));
    }
    Ok(listed)
}

fn upsert_key_of(event: &GoogleEvent) -> Option<&String> {
    event
        .extended_properties
        .as_ref()?
        .private
        .get(UPSERT_PROPERTY)
}

/// One agenda item, or `None` for an event Almanac did not write.
fn event_json(
    event: &GoogleEvent,
    calendar_id: &str,
    calendar: &str,
    series: usize,
    latest: &HashMap<String, DeliveryRecord>,
) -> Option<Value> {
    let key = upsert_key_of(event)?;
    let Some((start, event_time)) = event_start(&event.start) else {
        tracing::warn!(key = %key, "history: an event of Almanac's has a start the week agenda cannot read; it is left out");
        return None;
    };
    let source = key.split(':').next().unwrap_or(key).to_string();
    let record = latest.get(key.as_str());
    let delivered = record.map(|r| human(&r.at));
    let detail = match &delivered {
        Some(at) => format!("{source} · delivered {at}"),
        None => source.clone(),
    };
    Some(json!({
        "id": format!("{calendar_id}/{}", event.id.clone().unwrap_or_default()),
        "start": start,
        "label": event.summary,
        "series": series,
        "detail": detail,
        "source": source,
        "calendar": calendar,
        "event_time": event_time,
        "delivered": delivered,
        "result": record.map(|r| r.result),
    }))
}

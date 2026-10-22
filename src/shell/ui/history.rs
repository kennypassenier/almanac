//! The History page (/history) and its JSON routes: the deliveries-per-day
//! month heatmap with the picked day's deliveries, and the week agenda of
//! what Almanac wrote into the managed calendars.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use chassis::Dashboard;
use chrono::{DateTime, Datelike, Days, NaiveDate, SecondsFormat, TimeZone};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::calendar::{EventDateTime, GoogleEvent};
use crate::core::history::{self, DeliveryResult, ZONE};
use crate::shell::delivery::UPSERT_PROPERTY;
use crate::shell::history_store;
use crate::shell::ingest::AppState;

const PAGE_HTML: &str = include_str!("../../../templates/history.html");

/// Week colours the package offers (`--chart-1` to `--chart-5`).
const SERIES: usize = 5;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/history", get(page))
        .route("/history/api/days", get(days))
        .route("/history/api/day", get(day))
        .route("/history/api/week", get(week))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project("/history", PAGE_HTML, json!({})) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}

fn today(state: &AppState) -> String {
    history::day_key(&(state.now)()).unwrap_or_default()
}

/// `2026-10-19T08:16:00+00:00` as `19/10/2026 08:16` in Brussels.
fn human(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|at| at.with_timezone(&ZONE).format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_default()
}

/// Every recorded day's counts: the heatmap reads the whole history once.
async fn days(State(state): State<Arc<AppState>>) -> Result<Json<Value>, chassis::Error> {
    let counts = history_store::day_counts(&state).await?;
    Ok(Json(json!({
        "today": today(&state),
        "first_day": counts.first().map(|d| d.date.as_str()),
        "days": counts.iter().map(|d| json!({
            "date": d.date,
            "accepted": d.accepted,
            "delivered": d.delivered,
            "failed": d.failed,
        })).collect::<Vec<_>>(),
    })))
}

/// Calendar id to display name, asking Google only when a managed
/// calendar is missing from the remembered list.
async fn calendar_names(state: &AppState) -> HashMap<String, String> {
    let mut names: HashMap<String, String> = state.calendar_options().into_iter().collect();
    let missing = state
        .profiles()
        .values()
        .any(|p| !names.contains_key(&p.target_calendar_id));
    if missing && let Ok(list) = state.client.list_calendars().await {
        state.remember_calendars(&list);
        names = list.into_iter().collect();
    }
    names
}

#[derive(Deserialize)]
struct DayQuery {
    date: String,
}

/// One day's counts and deliveries.
async fn day(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DayQuery>,
) -> Result<Json<Value>, chassis::Error> {
    if !history::is_day(&q.date) {
        return Err(chassis::Error::invalid(
            "that is not a day",
            "pick a day in the calendar",
        ));
    }
    let counts = history_store::day_counts(&state).await?;
    let names = calendar_names(&state).await;
    let record = counts.iter().find(|d| d.date == q.date);
    let deliveries = history_store::day_rows(&state, &q.date).await?;
    let rows: Vec<Value> = deliveries
        .iter()
        .map(|r| {
            json!({
                "id": r.entry_id,
                "delivered": human(&r.at),
                "title": r.title,
                "source": r.source,
                "calendar": names.get(&r.calendar_id)
                    .cloned()
                    .unwrap_or_else(|| "A calendar that is no longer there".to_string()),
                "result": r.result,
                "reason": r.reason,
            })
        })
        .collect();
    let count = |result: DeliveryResult| deliveries.iter().filter(|r| r.result == result).count();
    Ok(Json(json!({
        "date": q.date,
        "first_day": counts.first().map(|d| d.date.as_str()),
        "accepted": record.map_or(0, |d| d.accepted),
        "created": count(DeliveryResult::Created),
        "updated": count(DeliveryResult::Updated),
        "failed": record.map_or(0, |d| d.failed),
        "rows": rows,
    })))
}

#[derive(Deserialize)]
struct WeekQuery {
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
async fn week(
    State(state): State<Arc<AppState>>,
    Query(q): Query<WeekQuery>,
) -> Result<Json<Value>, chassis::Error> {
    let first = match q.start.as_deref() {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
            chassis::Error::invalid("that is not a day", "use the week buttons to move")
        })?,
        None => {
            let now = NaiveDate::parse_from_str(&today(&state), "%Y-%m-%d").unwrap_or_default();
            now - Days::new(u64::from(now.weekday().num_days_from_monday()))
        }
    };
    let (time_min, time_max) = (midnight(first), midnight(first + Days::new(7)));

    let names = calendar_names(&state).await;
    let mut managed: Vec<String> = state
        .profiles()
        .values()
        .map(|p| p.target_calendar_id.clone())
        .collect();
    managed.sort();
    managed.dedup();
    // One colour per calendar, stable across weeks: by name, then id.
    managed.sort_by_key(|id| (names.get(id).cloned().unwrap_or_default(), id.clone()));
    let name_of = |id: &str| {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "A calendar that is no longer there".to_string())
    };

    let mut listed: Vec<(usize, &String, Vec<GoogleEvent>)> = Vec::new();
    for (index, id) in managed.iter().enumerate() {
        let found: Vec<GoogleEvent> = state
            .client
            .list_events_between(id, &time_min, &time_max)
            .await
            .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
        listed.push((index, id, found));
    }
    // Only this week's keys are looked up, so the page never reads the
    // whole history for the "delivered" lines.
    let keys: Vec<String> = listed
        .iter()
        .flat_map(|(_, _, found)| found.iter())
        .filter_map(|e| e.extended_properties.as_ref()?.private.get(UPSERT_PROPERTY))
        .cloned()
        .collect();
    let latest = history_store::latest_by_key(&state, keys).await?;

    let mut events = Vec::new();
    let mut used = std::collections::BTreeMap::new();
    for (index, id, found) in listed {
        let series = index % SERIES + 1;
        for event in found {
            let Some(key) = event
                .extended_properties
                .as_ref()
                .and_then(|p| p.private.get(UPSERT_PROPERTY))
            else {
                continue;
            };
            let Some((start, event_time)) = event_start(&event.start) else {
                continue;
            };
            let source = key.split(':').next().unwrap_or(key).to_string();
            let record = latest.get(key.as_str());
            let delivered = record.map(|r| human(&r.at));
            let detail = match &delivered {
                Some(at) => format!("{source} · delivered {at}"),
                None => source.clone(),
            };
            used.insert(series, name_of(id));
            events.push(json!({
                "id": format!("{id}/{}", event.id.clone().unwrap_or_default()),
                "start": start,
                "label": event.summary,
                "series": series,
                "detail": detail,
                "source": source,
                "calendar": name_of(id),
                "event_time": event_time,
                "delivered": delivered,
                "result": record.map(|r| r.result),
            }));
        }
    }
    Ok(Json(json!({
        "start": first.format("%Y-%m-%d").to_string(),
        "events": events,
        "calendars": used.into_iter()
            .map(|(series, name)| json!({ "series": series, "name": name }))
            .collect::<Vec<_>>(),
    })))
}

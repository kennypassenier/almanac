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
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::history::{self, DeliveryResult, ZONE};
use crate::shell::history_store;
use crate::shell::state::AppState;

const PAGE_HTML: &str = include_str!("../../../../templates/history.html");

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

mod week;

use week::week;

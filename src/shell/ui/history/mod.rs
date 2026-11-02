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
pub(super) struct CalendarNames {
    names: HashMap<String, String>,
    /// False when Google was needed and did not answer: a missing name
    /// then means "not known now", not "the calendar is gone".
    complete: bool,
}

impl CalendarNames {
    /// The calendar's name, or words that say why there is none.
    pub(super) fn name_of(&self, id: &str) -> String {
        match self.names.get(id) {
            Some(name) => name.clone(),
            None if self.complete => "A calendar that is no longer there".to_string(),
            None => "Not known while Google does not answer".to_string(),
        }
    }

    pub(super) fn map(&self) -> &HashMap<String, String> {
        &self.names
    }
}

async fn calendar_names(state: &AppState) -> CalendarNames {
    let names: HashMap<String, String> = state.calendar_options().into_iter().collect();
    let missing = state
        .profiles()
        .values()
        .any(|p| !names.contains_key(&p.target_calendar_id));
    if !missing {
        return CalendarNames {
            names,
            complete: true,
        };
    }
    match state.client.list_calendars().await {
        Ok(list) => {
            state.remember_calendars(&list);
            CalendarNames {
                names: list.into_iter().collect(),
                complete: true,
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "history: could not list the calendars; names show as not known");
            CalendarNames {
                names,
                complete: false,
            }
        }
    }
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
    let read = history_store::day_rows(&state, &q.date).await?;
    let deliveries = read.rows;
    let rows: Vec<Value> = deliveries
        .iter()
        .map(|r| {
            json!({
                "id": r.entry_id,
                "delivered": human(&r.at),
                "title": r.title,
                "source": r.source,
                "calendar": names.name_of(&r.calendar_id),
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
        "unreadable": read.unreadable,
    })))
}

mod week;

use week::week;

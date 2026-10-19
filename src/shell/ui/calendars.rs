//! JSON routes for the calendars page's kp-themes components.
//!
//! The page itself renders at once and asks these routes for its data, so a
//! slow Google never holds the page back: `GET /calendars/data/google` lists
//! the calendars (the only call that waits on Google), `GET
//! /calendars/data/profiles` reads the profile files and the delivery ring
//! (fast, local). Making and deleting calendars and deleting an unusable
//! profile file are JSON calls too; the old form posts in `dashboard.rs` run
//! the same functions, so a bookmark behaves like the page.
//!
//! User-facing sentences here are plain: no paths, no ids, no raw upstream
//! errors (those go to the log).

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chassis::Error;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::observability::RouteOutcome;
use crate::shell::ingest::{AppState, HISTORY_CAPACITY};

/// Hours the trend tile covers.
const TREND_HOURS: usize = 24;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/calendars/data/google", get(google_data))
        .route("/calendars/data/profiles", get(profiles_data))
        .route("/calendars/make", post(make))
        .route("/calendars/remove", post(remove))
        .route("/profiles/remove", post(remove_file))
}

// ── reading ────────────────────────────────────────────────────────────

/// Lists the calendars the service account sees, the way the page always
/// did: through the tombstones and the just-created memory, because
/// Google's list lags a create or a delete by seconds.
pub(crate) async fn list_calendars(
    state: &AppState,
) -> Result<Vec<(String, String)>, crate::core::error::AlmanacError> {
    let listed = state.client.list_calendars().await?;
    let calendars = state.without_deleted_calendars(state.with_created_calendars(listed));
    state.remember_calendars(&calendars);
    Ok(calendars)
}

/// `GET /calendars/data/google`. A failed list is a 200 with `ok: false` and
/// the words to show, so the card can offer Retry without a transport error.
async fn google_data(State(state): State<Arc<AppState>>) -> Json<Value> {
    match list_calendars(&state).await {
        Ok(calendars) => Json(json!({
            "ok": true,
            "can_create": state.calendar_owner.is_some(),
            "calendars": calendars
                .iter()
                .map(|(id, name)| json!({ "id": id, "name": name }))
                .collect::<Vec<_>>(),
        })),
        Err(e) => {
            tracing::warn!(error = %e, "could not list the calendars for the page");
            Json(json!({
                "ok": false,
                "can_create": state.calendar_owner.is_some(),
                "error": "Google did not answer, so the calendars could not be listed.",
                "remedy": "Almanac keeps the events and delivers them when Google answers again.",
            }))
        }
    }
}

/// `GET /calendars/data/profiles`: the served files, the unusable ones with
/// their reason and fix, what each source delivered lately and the trend.
async fn profiles_data(State(state): State<Arc<AppState>>) -> Json<Value> {
    let loaded = crate::shell::profiles::load_all(&state.profiles_dir);
    let folder_ok = !loaded.unusable.iter().any(|u| u.path == state.profiles_dir);
    let unserved: Vec<Value> = loaded
        .unusable
        .iter()
        .filter(|u| u.path != state.profiles_dir)
        .map(|u| {
            let (why, remedy) = split_reason(&u.reason, &state.profiles_dir, &u.file_name());
            json!({ "file": u.file_name(), "why": why, "remedy": remedy })
        })
        .collect();

    // The ring holds the latest deliveries since start (at most
    // HISTORY_CAPACITY), newest first: it is the only per-source history
    // Almanac keeps, so "last used", "delivered" and the trend come from it.
    let routes: Vec<(String, Option<i64>, bool)> = state
        .routes
        .lock()
        .await
        .iter()
        .map(|r| {
            let at = chrono::DateTime::parse_from_rfc3339(&r.at)
                .ok()
                .map(|d| d.timestamp());
            (
                r.source_id.clone(),
                at,
                !matches!(r.outcome, RouteOutcome::Failed { .. }),
            )
        })
        .collect();

    let served_profiles = state.profiles();
    let mut ids: Vec<&String> = served_profiles.keys().collect();
    ids.sort();
    let mut sources = Vec::new();
    let mut served = Vec::new();
    for id in ids {
        let p = &served_profiles[id];
        let mine: Vec<&(String, Option<i64>, bool)> =
            routes.iter().filter(|r| &r.0 == id).collect();
        let delivered = mine.iter().filter(|r| r.2).count();
        let last_at = mine.iter().find(|r| r.2).and_then(|r| r.1);
        let last_failed = mine.first().is_some_and(|r| !r.2);
        sources.push(json!({
            "id": id,
            "calendar_id": p.target_calendar_id,
            "delivered": delivered,
            "last_at_ms": last_at.map(|s| s * 1000),
            "last_failed": last_failed,
        }));
        served.push(json!({
            "file": format!("{id}.toml"),
            "source": id,
            "calendar_id": p.target_calendar_id,
            "health": if last_failed { "warn" } else { "ok" },
        }));
    }

    let now = (state.now_unix)() as i64;
    let delivered_at: Vec<i64> = routes.iter().filter(|r| r.2).filter_map(|r| r.1).collect();
    let values = hourly_buckets(&delivered_at, now, TREND_HOURS);
    let start = now / 3600 * 3600 - (TREND_HOURS as i64 - 1) * 3600;
    let oldest = routes.iter().filter_map(|r| r.1).min();
    let truncated = routes.len() >= HISTORY_CAPACITY && oldest.is_some_and(|o| o > start);
    let total: u32 = values.iter().sum();
    let busiest = values
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0)
        .max_by_key(|(i, v)| (**v, *i))
        .map(|(i, v)| json!({ "count": v, "hour_ms": (start + i as i64 * 3600) * 1000 }));

    Json(json!({
        "folder_ok": folder_ok,
        "served": served,
        "unserved": unserved,
        "sources": sources,
        "trend": {
            "start_ms": start * 1000,
            "step_ms": 3_600_000,
            "values": values,
            "total": total,
            "busiest": busiest,
            "truncated": truncated,
        },
    }))
}

/// Deliveries per hour: the last `hours` whole hours ending with the one
/// that holds `now`, oldest first. Pure; a time outside the window counts
/// nowhere.
pub(crate) fn hourly_buckets(times: &[i64], now: i64, hours: usize) -> Vec<u32> {
    let first = now / 3600 * 3600 - (hours as i64 - 1) * 3600;
    let mut out = vec![0u32; hours];
    for t in times {
        let i = (t - first).div_euclid(3600);
        if (t - first) >= 0 && (i as usize) < hours {
            out[i as usize] += 1;
        }
    }
    out
}

/// Splits an unusable file's reason into what is wrong and what to do, in
/// two plain sentences. The loader writes "reason — remedy"; paths are
/// replaced by words so none reaches the page.
pub(crate) fn split_reason(reason: &str, dir: &std::path::Path, file: &str) -> (String, String) {
    let clean = reason
        .replace(&dir.join(file).display().to_string(), file)
        .replace(&dir.display().to_string(), "the profiles folder");
    let (why, remedy) = match clean.split_once(" — ") {
        Some((w, r)) => (w.trim().to_string(), r.trim().to_string()),
        None => (clean.trim().to_string(), String::new()),
    };
    let sentence = |s: &str| {
        let s = s.trim().trim_end_matches('.');
        let mut chars = s.chars();
        match chars.next() {
            Some(c) => format!("{}{}.", c.to_uppercase(), chars.as_str()),
            None => String::new(),
        }
    };
    let remedy = if remedy.is_empty() {
        "Fix the file and reload the profiles, or delete the file.".to_string()
    } else {
        format!("{} Or delete the file.", sentence(&remedy))
    };
    (sentence(&format!("{file}: {why}")), remedy)
}

// ── writing ────────────────────────────────────────────────────────────

/// Makes a calendar and shares it with the owner. Find-or-create, serialized
/// per name and consulted against what Almanac made moments ago: Google's
/// list lags a create by seconds, and two clicks inside that window would
/// both create.
pub(crate) async fn make_calendar(
    state: &AppState,
    name: &str,
) -> Result<(String, String, bool), Error> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid(
            "The calendar needs a name",
            "Type the name it should have in Google Calendar.",
        ));
    }
    let Some(owner) = state.calendar_owner.as_deref() else {
        return Err(Error::invalid(
            "Almanac cannot make calendars: the owner is not set",
            "Set ALMANAC_CALENDAR_OWNER, so a new calendar is shared with someone who can see it.",
        ));
    };
    let lock = state.locks.for_key(&format!("calendar:{name}")).await;
    let _guard = lock.lock().await;
    if let Some(id) = state.remembered_calendar(name) {
        tracing::info!(
            calendar = %name,
            id = %id,
            "a calendar with that name was just created; reusing it rather than making a second"
        );
        return Ok((id, name.to_string(), false));
    }
    match state.client.ensure_calendar(name, owner).await {
        Ok((id, created)) => {
            if created {
                state.remember_created_calendar(name, &id);
                tracing::info!(
                    calendar = %name,
                    id = %id,
                    shared_with = %owner,
                    role = "owner",
                    "created a calendar from the dashboard and shared it"
                );
            }
            Ok((id, name.to_string(), created))
        }
        Err(e) => {
            tracing::warn!(error = %e, calendar = %name, "could not make a calendar");
            Err(Error::dependency(
                "Google refused the request, so the calendar was not made",
                "Check that the service account may make calendars, then try again.",
            ))
        }
    }
}

/// Removes a calendar and everything on it. Refused while a source still
/// writes to it, checked here as well as in the page: a source can be
/// added between the render and the click.
pub(crate) async fn remove_calendar(state: &AppState, calendar_id: &str) -> Result<String, Error> {
    let name = state.calendar_name(calendar_id);
    let mut users: Vec<String> = state
        .profiles()
        .values()
        .filter(|p| p.target_calendar_id == calendar_id)
        .map(|p| p.source_id.clone())
        .collect();
    users.sort();
    if !users.is_empty() {
        return Err(Error::invalid(
            format!("{name} was not deleted: {} writes to it", users.join(", ")),
            "Delete that source first, on the Sources page.",
        ));
    }
    match state.client.delete_calendar(calendar_id).await {
        Ok(()) => {
            state.remember_deleted_calendar(calendar_id);
            state.forget_created_calendar(calendar_id);
            tracing::info!(calendar_id = %calendar_id, "deleted a calendar from the dashboard");
            Ok(name)
        }
        Err(e) => {
            tracing::warn!(error = %e, calendar_id = %calendar_id, "could not delete a calendar");
            Err(Error::dependency(
                format!("Google refused the request, so {name} was not deleted"),
                "Try again in a moment.",
            ))
        }
    }
}

/// Removes a profile file the service cannot use (addressed by file name: a
/// broken profile often has no readable source id) and reloads the set.
pub(crate) fn remove_unusable_file(state: &AppState, file_name: &str) -> Result<(), Error> {
    let unusable = crate::shell::profiles::load_all(&state.profiles_dir).unusable;
    if !unusable.iter().any(|u| u.file_name() == file_name) {
        return Err(Error::new(
            chassis::Kind::NotFound,
            format!("{file_name} is not an unused profile file (any more)"),
            "Reload the page; it may already be gone.",
        ));
    }
    match crate::shell::profiles::delete_file(&state.profiles_dir, file_name) {
        Ok(removed) => {
            state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
            tracing::info!(removed = %removed.display(), "deleted an unusable profile from the dashboard");
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = %e, file = %file_name, "could not delete an unusable profile");
            Err(Error::dependency(
                format!("{file_name} could not be deleted"),
                "Check that the profiles folder is writable, then try again.",
            ))
        }
    }
}

/// What the reload answers: how many files are served and how many are not.
pub(crate) fn reload_summary(state: &AppState) -> Value {
    let profiles = crate::shell::profiles::load_map(&state.profiles_dir);
    let ids: HashSet<&String> = profiles.keys().collect();
    let served = ids.len();
    tracing::info!(count = served, "reloaded profiles from the dashboard");
    state.set_profiles(profiles);
    let unserved = crate::shell::profiles::load_all(&state.profiles_dir)
        .unusable
        .iter()
        .filter(|u| u.path != state.profiles_dir)
        .count();
    json!({ "served": served, "unserved": unserved })
}

#[derive(Deserialize)]
struct MakeBody {
    name: String,
}

#[derive(Deserialize)]
struct RemoveBody {
    id: String,
}

#[derive(Deserialize)]
struct RemoveFileBody {
    file: String,
}

/// `POST /calendars/make` `{ "name": … }`.
async fn make(
    State(state): State<Arc<AppState>>,
    Json(body): Json<MakeBody>,
) -> Result<Json<Value>, Error> {
    let (id, name, created) = make_calendar(&state, &body.name).await?;
    Ok(Json(json!({ "id": id, "name": name, "created": created })))
}

/// `POST /calendars/remove` `{ "id": … }`.
async fn remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RemoveBody>,
) -> Result<Json<Value>, Error> {
    let name = remove_calendar(&state, &body.id).await?;
    Ok(Json(json!({ "name": name })))
}

/// `POST /profiles/remove` `{ "file": … }`.
async fn remove_file(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RemoveFileBody>,
) -> Result<Json<Value>, Error> {
    remove_unusable_file(&state, &body.file)?;
    Ok(Json(json!({ "file": body.file })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_count_the_last_hours_oldest_first() {
        let now = 10 * 3600 + 120;
        let v = hourly_buckets(&[now, now - 60, 9 * 3600, 2 * 3600, 0], now, 3);
        assert_eq!(v, vec![0, 1, 2]);
    }

    #[test]
    fn a_reason_splits_into_why_and_what_now_without_a_path() {
        let dir = std::path::Path::new("/srv/profiles");
        let (why, remedy) = split_reason(
            "written for schema_version 1; this build reads 2 — reduce it to two fields",
            dir,
            "old.toml",
        );
        assert_eq!(
            why,
            "Old.toml: written for schema_version 1; this build reads 2."
        );
        assert_eq!(remedy, "Reduce it to two fields. Or delete the file.");
        let (_, plain) = split_reason("could not be read", dir, "x.toml");
        assert!(plain.contains("reload the profiles"));
    }
}

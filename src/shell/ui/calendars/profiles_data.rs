//! `GET /calendars/data/profiles`: the served profile files, the unusable
//! ones with their reason and fix, what each source delivered lately and
//! the 24-hour trend.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::observability::RouteOutcome;
use crate::core::profile::Profile;
use crate::shell::state::{AppState, HISTORY_CAPACITY};

/// Hours the trend tile covers.
const TREND_HOURS: usize = 24;
const SECONDS_PER_HOUR: i64 = 3600;

/// One delivery from the route ring: which source, when (Unix seconds,
/// when the time reads), and whether it was delivered.
struct Point {
    source_id: String,
    at: Option<i64>,
    delivered: bool,
}

pub(super) async fn profiles_data(state: Arc<AppState>) -> Value {
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
    let points = route_points(&state).await;
    let (sources, served) = per_source(&state.profiles(), &points);
    let now = (state.now_unix)() as i64;
    json!({
        "folder_ok": folder_ok,
        "served": served,
        "unserved": unserved,
        "sources": sources,
        "trend": trend(&points, now),
    })
}

/// The ring holds the latest deliveries since start (at most
/// HISTORY_CAPACITY), newest first: it is the only per-source history
/// Almanac keeps, so "last used", "delivered" and the trend come from it.
async fn route_points(state: &AppState) -> Vec<Point> {
    state
        .routes
        .lock()
        .await
        .iter()
        .map(|r| Point {
            source_id: r.source_id.clone(),
            at: match chrono::DateTime::parse_from_rfc3339(&r.at) {
                Ok(at) => Some(at.timestamp()),
                Err(e) => {
                    tracing::warn!(at = %r.at, error = %e, "a delivery record carries an unreadable time; it counts in no hour");
                    None
                }
            },
            delivered: !matches!(r.outcome, RouteOutcome::Failed { .. }),
        })
        .collect()
}

/// Per served source, sorted by id: the row of the sources table and the
/// row of the served-files list.
fn per_source(profiles: &HashMap<String, Profile>, points: &[Point]) -> (Vec<Value>, Vec<Value>) {
    let mut ids: Vec<&String> = profiles.keys().collect();
    ids.sort();
    let mut sources = Vec::new();
    let mut served = Vec::new();
    for id in ids {
        let profile = &profiles[id];
        let mine: Vec<&Point> = points.iter().filter(|p| &p.source_id == id).collect();
        let delivered = mine.iter().filter(|p| p.delivered).count();
        let last_at = mine.iter().find(|p| p.delivered).and_then(|p| p.at);
        let last_failed = mine.first().is_some_and(|p| !p.delivered);
        sources.push(json!({
            "id": id,
            "calendar_id": profile.target_calendar_id,
            "delivered": delivered,
            "last_at_ms": last_at.map(|s| s * 1000),
            "last_failed": last_failed,
        }));
        served.push(json!({
            "file": format!("{id}.toml"),
            "source": id,
            "calendar_id": profile.target_calendar_id,
            "health": if last_failed { "warn" } else { "ok" },
        }));
    }
    (sources, served)
}

/// Deliveries per hour over the last TREND_HOURS, the busiest hour, and
/// whether the ring is too short to cover the whole window.
fn trend(points: &[Point], now: i64) -> Value {
    let delivered_at: Vec<i64> = points
        .iter()
        .filter(|p| p.delivered)
        .filter_map(|p| p.at)
        .collect();
    let values = hourly_buckets(&delivered_at, now, TREND_HOURS);
    let start =
        now / SECONDS_PER_HOUR * SECONDS_PER_HOUR - (TREND_HOURS as i64 - 1) * SECONDS_PER_HOUR;
    let oldest = points.iter().filter_map(|p| p.at).min();
    let truncated = points.len() >= HISTORY_CAPACITY && oldest.is_some_and(|o| o > start);
    let total: u32 = values.iter().sum();
    let busiest = values
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0)
        .max_by_key(|(i, v)| (**v, *i))
        .map(
            |(i, v)| json!({ "count": v, "hour_ms": (start + i as i64 * SECONDS_PER_HOUR) * 1000 }),
        );
    json!({
        "start_ms": start * 1000,
        "step_ms": SECONDS_PER_HOUR * 1000,
        "values": values,
        "total": total,
        "busiest": busiest,
        "truncated": truncated,
        "ring_size": HISTORY_CAPACITY,
    })
}

/// Deliveries per hour: the last `hours` whole hours ending with the one
/// that holds `now`, oldest first. Pure; a time outside the window counts
/// nowhere.
pub(crate) fn hourly_buckets(times: &[i64], now: i64, hours: usize) -> Vec<u32> {
    let first = now / SECONDS_PER_HOUR * SECONDS_PER_HOUR - (hours as i64 - 1) * SECONDS_PER_HOUR;
    let mut out = vec![0u32; hours];
    for t in times {
        let i = (t - first).div_euclid(SECONDS_PER_HOUR);
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

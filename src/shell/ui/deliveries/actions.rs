//! Retry and dismiss for the entries Almanac set aside: two JSON routes
//! behind the dashboard login, used per row and for a selection.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chassis::Error;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::shell::journal::RequeueError;
use crate::shell::state::AppState;

#[derive(Deserialize)]
pub(super) struct IdsBody {
    ids: Vec<String>,
}

/// `POST /deliveries/retry` `{ "ids": [...] }` -> `{ "retried": n }`.
pub(super) async fn retry(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IdsBody>,
) -> Result<Json<Value>, Error> {
    check_not_empty(&body.ids)?;
    let retried = state
        .journal
        .retry_dead(&body.ids, &(state.now)())
        .await
        .map_err(refusal)?;
    tracing::info!(retried, "set-aside events queued again from the dashboard");
    Ok(Json(json!({ "retried": retried })))
}

/// `POST /deliveries/dismiss` `{ "ids": [...] }` -> `{ "dismissed": n }`.
pub(super) async fn dismiss(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IdsBody>,
) -> Result<Json<Value>, Error> {
    check_not_empty(&body.ids)?;
    let dismissed = state
        .journal
        .dismiss_dead(&body.ids)
        .await
        .map_err(refusal)?;
    tracing::info!(
        dismissed,
        "set-aside events marked handled from the dashboard"
    );
    Ok(Json(json!({ "dismissed": dismissed })))
}

fn check_not_empty(ids: &[String]) -> Result<(), Error> {
    if ids.is_empty() {
        return Err(Error::invalid(
            "No event was chosen",
            "Choose at least one set-aside event.",
        ));
    }
    Ok(())
}

/// A refusal in plain words: no ids, no paths. The detail goes to the log.
fn refusal(e: RequeueError) -> Error {
    match e {
        RequeueError::NotSetAside(ids) => {
            tracing::info!(?ids, "refused: these events are not set aside");
            let many = ids.len() != 1;
            Error::invalid(
                if many {
                    "Some of these events are no longer set aside"
                } else {
                    "This event is no longer set aside"
                },
                "Nothing was changed. Reload the page to see what is left.",
            )
        }
        RequeueError::Journal(e) => {
            tracing::error!(error = %e, "could not update the journal");
            Error::internal(
                "Almanac could not write to its journal, so nothing was changed",
                "Try again in a moment. If it keeps failing, check the disk space of the service.",
            )
        }
    }
}

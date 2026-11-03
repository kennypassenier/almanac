//! The shared state a start builds, and the one-time token import that
//! runs before the kit opens its client store.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use almanac::core::paths::Paths;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::journal::Journal;
use almanac::shell::kit::import_source_tokens;
use almanac::shell::state::AppState;

/// The shared state every route and the worker read.
pub fn build_state(
    env: &BTreeMap<String, String>,
    paths: &Paths,
    profiles: Vec<almanac::core::profile::Profile>,
    http: &reqwest::Client,
    tokens: &Arc<TokenManager>,
    metrics: Arc<almanac::core::metrics::Metrics>,
) -> Result<Arc<AppState>, almanac::core::error::AlmanacError> {
    use almanac::core::google_base::{GOOGLE_API_BASE_ENV, api_root};
    let root = api_root(env.get(GOOGLE_API_BASE_ENV).map(String::as_str))?;
    let profiles: HashMap<String, _> = profiles
        .into_iter()
        .map(|p| (p.source_id.clone(), p))
        .collect();
    Ok(Arc::new(
        AppState::new(
            profiles,
            Journal::new(paths.journal.clone()),
            GoogleCalendarClient::with_api_root(http.clone(), Arc::clone(tokens), &root),
        )
        .with_profiles_dir(paths.profiles_dir.clone())
        .with_calendar_owner(
            env.get("ALMANAC_CALENDAR_OWNER")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
        )
        .with_metrics(metrics),
    ))
}

/// A2-1: the per-source tokens are the kit's clients. The 3.x store is
/// read once, on the first start of 4.0.0, and copied unchanged into the
/// kit's sealed file, so every source keeps its token.
pub async fn import_tokens(
    env: &BTreeMap<String, String>,
    state_dir: &std::path::Path,
    paths: &Paths,
) -> Result<(), String> {
    let secret = env
        .get("ALMANAC_SECRET_KEY")
        .map(String::as_str)
        .unwrap_or_default();
    match import_source_tokens(state_dir, &paths.token_store, secret).await {
        Ok(0) => Ok(()),
        Ok(count) => {
            eprintln!(
                "almanac: imported {count} source token(s) from the 3.x store into the kit's client store; every source keeps its token"
            );
            Ok(())
        }
        Err(e) => Err(format!("{e:#}")),
    }
}

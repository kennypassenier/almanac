//! Who may post as which source (K6).

use axum::http::StatusCode;
use chassis::Caller;

use super::{AppState, Reply, error};
use crate::core::profile::Profile;

/// Resolves the profile for `source_id` and decides whether the caller
/// may post as that source.
///
/// 4.0.0: the kit's door already checked the bearer token and named the
/// caller; what is left is whether THIS token belongs to THIS source — a
/// client under the source's own name, or the admin (K6). An unknown
/// source and a foreign token both answer 401 with the same body:
/// distinguishing them would tell a caller which source ids exist.
pub(super) async fn authenticate(
    state: &AppState,
    source_id: &str,
    caller: &Caller,
) -> Result<Profile, Reply> {
    let unauthorized = || {
        error(
            StatusCode::UNAUTHORIZED,
            "unknown source or invalid token",
            "post with the token issued for this source on the Sources page (the client's name is the source id)",
        )
    };
    let profiles = state.profiles();
    let Some(profile) = profiles.get(source_id) else {
        return Err(unauthorized());
    };
    match caller {
        Caller::Admin => Ok(profile.clone()),
        Caller::Client { name, .. } if name == source_id => Ok(profile.clone()),
        Caller::Client { .. } => Err(unauthorized()),
    }
}

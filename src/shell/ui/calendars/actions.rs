//! Making and deleting calendars, deleting an unusable profile file and
//! reloading the profiles: shared by the JSON routes and the form posts.

use std::collections::HashSet;

use chassis::Error;
use serde_json::{Value, json};

use crate::shell::state::AppState;

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
    ensure_on_google(state, name, owner).await
}

/// Finds or creates the calendar on Google and remembers a new one, so
/// the next click inside Google's lag reuses it.
async fn ensure_on_google(
    state: &AppState,
    name: &str,
    owner: &str,
) -> Result<(String, String, bool), Error> {
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

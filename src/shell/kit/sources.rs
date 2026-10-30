//! The kit's clients are Almanac's sources: the Calendar column, and the
//! profile written when a source is issued and removed when it is deleted.

use std::sync::Arc;

use crate::shell::state::AppState;

/// The Profiles section on the kit's status page (A2-2): which sources
/// are served, and where their events go.
/// The calendar a source writes to, by name (K16 column, 4.0.2).
pub struct TargetCalendar(pub Arc<AppState>);

impl chassis::shell::dashboard::ClientColumn for TargetCalendar {
    fn title(&self) -> String {
        // Not "Calendar": chassis-rs 2.0.0 (feat-clients-2) renders the
        // "calendar" form field's own raw stored value under that header
        // automatically now, so a second "Calendar" column would sit next
        // to it with nothing but the header to tell them apart. This one
        // stays because it is the friendly name (K16); the kit's shows
        // the id, mirroring the Calendars page's own id-behind-a-toggle.
        "Calendar name".into()
    }
    fn cell(&self, client: &chassis::shell::clients_api::ClientView) -> String {
        match self.0.profiles().get(&client.name) {
            Some(p) => crate::core::html::escape(&self.0.calendar_name(&p.target_calendar_id)),
            None => "<span class=\"text-secondary italic\">no profile</span>".into(),
        }
    }
}

/// S1 (4.0.2): a source is a name AND a calendar, made in one go on the
/// kit's Sources page. Refusing here issues no token.
pub fn create_source(
    state: &AppState,
    source_id: &str,
    calendar: &str,
) -> Result<(), chassis::Error> {
    let source_id = source_id.trim();
    let calendar = calendar.trim();
    // A source that already has a profile (a 3.x source getting its token
    // issued again, or one whose profile was placed by hand) keeps it; the
    // calendar chosen on the form does not overwrite a profile on disk.
    if state.profiles().contains_key(source_id) {
        tracing::info!(source_id = %source_id, "token issued for a source that already has a profile");
        return Ok(());
    }
    if !crate::core::profile::source_id_is_safe(source_id) {
        return Err(chassis::Error::invalid(
            format!("\"{source_id}\" cannot be a source name"),
            "use letters, digits, '.', '-' and '_', and do not start with a dot",
        ));
    }
    if calendar.is_empty() {
        return Err(chassis::Error::invalid(
            "no calendar chosen",
            "pick the calendar this source writes to",
        ));
    }
    let toml = crate::core::profile::default_profile_toml(source_id, calendar);
    crate::shell::profiles::save_new(&state.profiles_dir, &toml)
        .map_err(|e| chassis::Error::invalid(e.to_string(), e.remedy().to_string()))?;
    // Reload from disk rather than inserting the parsed profile: reading
    // it back is the only proof that what was written can be read again.
    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(source_id = %source_id, "added a source from the Sources page");
    Ok(())
}

/// The other half of S1, asked before the kit deletes the client: the
/// profile goes with it — unless the journal still holds events for it,
/// which need the profile to be delivered; then the delete is refused and
/// the events it already put on the calendar stay either way (Kenny,
/// 2026-09-03).
pub fn remove_source(state: &AppState, source_id: &str) -> Result<(), chassis::Error> {
    if !state.profiles().contains_key(source_id) {
        return Ok(());
    }
    let waiting = state
        .journal
        .pending()
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?
        .iter()
        .filter(|e| e.source_id == source_id)
        .count();
    if waiting > 0 {
        return Err(chassis::Error::invalid(
            format!("{source_id} still has {waiting} event(s) waiting to be delivered"),
            "wait for the queue to drain, or fix whatever is blocking delivery, and delete it then",
        ));
    }
    let removed = crate::shell::profiles::delete(&state.profiles_dir, source_id)
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(source_id = %source_id, removed = %removed.display(), "source profile removed with its client");
    Ok(())
}

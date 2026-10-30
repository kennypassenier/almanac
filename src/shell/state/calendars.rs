//! The calendar-lag memory on [`AppState`] (K24).
//!
//! Google's calendar list is eventually consistent, so after a create or
//! a delete the very next list can still show the old truth. Almanac
//! remembers what it just did and corrects Google's answer with it; each
//! memory clears itself as soon as Google's own list agrees, so none of
//! it grows or outlives the disagreement. Also holds the plain
//! `(id, name)` cache the synchronous renderers use.

use super::AppState;

/// Locks a std mutex, taking the data even if a panicking holder
/// poisoned it: every value guarded here is a plain cache that is valid
/// after any partial update, and refusing to read it would turn one
/// panic into a permanently broken dashboard.
fn locked<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl AppState {
    /// Remembers that a calendar was deleted, so the page rendered
    /// straight afterwards does not show it (K24).
    pub fn remember_deleted_calendar(&self, calendar_id: &str) {
        self.deleted_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(calendar_id.to_string());
    }

    /// Remember what Google listed, so the synchronous renderers (the kit's
    /// issue form, the Target-calendar column) have names to show.
    pub fn remember_calendars(&self, calendars: &[(String, String)]) {
        *locked(&self.calendar_names) = calendars.to_vec();
    }

    /// The remembered calendars as `(id, name)` for a dropdown.
    pub fn calendar_options(&self) -> Vec<(String, String)> {
        locked(&self.calendar_names).clone()
    }

    /// A calendar's name when it is known; the id otherwise, so a row never
    /// shows nothing.
    pub fn calendar_name(&self, id: &str) -> String {
        locked(&self.calendar_names)
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Filters Google's list through what almanac knows it deleted, and
    /// forgets the ids Google has caught up on.
    pub fn without_deleted_calendars(
        &self,
        calendars: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let mut deleted = self
            .deleted_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if deleted.is_empty() {
            return calendars;
        }
        let listed: std::collections::HashSet<&str> =
            calendars.iter().map(|(id, _)| id.as_str()).collect();
        // Anything Google no longer lists has caught up; drop it here
        // so this set stays as small as the disagreement is.
        deleted.retain(|id| listed.contains(id.as_str()));
        calendars
            .into_iter()
            .filter(|(id, _)| !deleted.contains(id))
            .collect()
    }

    /// Remembers a calendar almanac just made, so the next few seconds
    /// do not make a second one with the same name (K24).
    pub fn remember_created_calendar(&self, summary: &str, calendar_id: &str) {
        self.created_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(summary.to_string(), calendar_id.to_string());
    }

    /// The id of a calendar almanac made under this name and Google has
    /// not listed yet, if there is one.
    pub fn remembered_calendar(&self, summary: &str) -> Option<String> {
        self.created_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(summary)
            .cloned()
    }

    /// Drops the memory of a created calendar by id.
    ///
    /// Called when one is deleted: without this, a calendar made and
    /// removed inside the same lag window would keep being added back
    /// to the rendered list by the very memory that was meant to make
    /// it appear.
    pub fn forget_created_calendar(&self, calendar_id: &str) {
        self.created_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, id| id != calendar_id);
    }

    /// Adds what almanac made and Google has not listed yet, and
    /// forgets the ones it now lists.
    ///
    /// The absence is as misleading as the stale presence was: a
    /// calendar created a second ago is missing from the page that
    /// renders next, which reads as "it did not work".
    pub fn with_created_calendars(
        &self,
        mut calendars: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let mut created = self
            .created_calendars
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if created.is_empty() {
            return calendars;
        }
        let listed: std::collections::HashSet<&str> =
            calendars.iter().map(|(id, _)| id.as_str()).collect();
        let missing: Vec<(String, String)> = created
            .iter()
            .filter(|(_, id)| !listed.contains(id.as_str()))
            .map(|(summary, id)| (id.clone(), summary.clone()))
            .collect();
        // Google has caught up on the rest; keeping them would mean
        // holding a name that could since have been renamed there.
        created.retain(|_, id| !listed.contains(id.as_str()));
        calendars.extend(missing);
        calendars.sort_by(|a, b| a.1.cmp(&b.1));
        calendars
    }
}

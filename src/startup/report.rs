//! The lines a start writes once logging is up: compatibility warnings,
//! the profiles that loaded and those that did not, and what the
//! journal still holds.

use std::path::PathBuf;

use almanac::shell::profiles::Unusable;

/// Everything checked before the bind that the log should say.
pub struct StartupReport {
    pub warnings: Vec<String>,
    pub unusable: Vec<Unusable>,
    pub profile_names: Vec<String>,
    pub profiles_dir: PathBuf,
    pub pending: usize,
    pub journal_path: PathBuf,
}

impl StartupReport {
    pub fn log(&self) {
        for warning in &self.warnings {
            tracing::warn!("{warning}");
        }
        self.log_profiles();
        if self.pending > 0 {
            tracing::info!(
                count = self.pending,
                journal = %self.journal_path.display(),
                "journal holds undelivered entries from a previous run; they go out first"
            );
        }
    }

    /// One line per unusable file, at error level: the quietest possible
    /// failure would be a count that simply came out lower than expected.
    /// They are also listed on the dashboard, where they can be deleted —
    /// which is why refusing to start over one would be exactly the
    /// wrong move.
    fn log_profiles(&self) {
        for unusable in &self.unusable {
            tracing::error!(
                path = %unusable.path.display(),
                reason = %unusable.reason,
                "a profile could not be used; this source is not being served"
            );
        }
        if self.profile_names.is_empty() {
            tracing::warn!(
                directory = %self.profiles_dir.display(),
                unusable = self.unusable.len(),
                "no usable mapping profiles — almanac is serving no sources; add one from /sources"
            );
        } else {
            tracing::info!(
                count = self.profile_names.len(),
                unusable = self.unusable.len(),
                sources = ?self.profile_names,
                "loaded mapping profiles"
            );
        }
    }
}

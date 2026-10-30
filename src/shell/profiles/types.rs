//! What a profiles directory yielded.

use std::path::PathBuf;

use crate::core::profile::Profile;

/// A profile file the service cannot use, and why.
///
/// Kept rather than thrown: the dashboard lists these so a person can
/// see what is not being served and delete it, and a file nobody can
/// see is a source that stopped working silently.
#[derive(Debug, Clone, PartialEq)]
pub struct Unusable {
    pub path: PathBuf,
    /// One sentence, in the shape every other error here takes: what is
    /// wrong, and what to do about it.
    pub reason: String,
}

impl Unusable {
    /// The file name, which is what the dashboard addresses it by — a
    /// broken profile may have no readable `source_id` at all.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// What a profiles directory yielded: what loaded, and what could not
/// be used.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loaded {
    pub profiles: Vec<Profile>,
    pub unusable: Vec<Unusable>,
}

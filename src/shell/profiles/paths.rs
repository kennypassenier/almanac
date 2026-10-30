//! The on-disk naming of profile files, shared by load, save and delete.

use std::path::{Path, PathBuf};

/// Extension of a profile file (without the dot).
pub(super) const PROFILE_EXT: &str = "toml";
/// Extension of the temporary file a save is written to before the rename.
pub(super) const TMP_EXT: &str = "toml.tmp";

/// `<dir>/<source_id>.toml`. Safe because `Profile::parse` has already
/// refused any source_id that is not filename-shaped
/// (`core::profile::source_id_is_safe`); this function is never reached
/// with an unvalidated id.
pub(super) fn profile_path(dir: &Path, source_id: &str) -> PathBuf {
    dir.join(format!("{source_id}.{PROFILE_EXT}"))
}

/// A write-permission remedy naming the profiles directory.
pub(super) fn writable_remedy(dir: &Path) -> String {
    format!(
        "check the profiles directory {} is writable by the almanac user",
        dir.display()
    )
}

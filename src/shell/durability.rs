//! One helper, shared by everything that replaces a file atomically.
//!
//! Writing a temp file, fsyncing it and renaming it over the target is
//! only half of the guarantee: the rename itself lives in the parent
//! directory, and on a real power cut an unsynced directory entry can
//! come back pointing at the old file — or at nothing. The journal's
//! compaction, the encrypted token store and the self-updater all do
//! that same dance, so they share this rather than each carrying their
//! own copy (the token store had the only one, and the journal was
//! missing it entirely).

use std::fs::File;
use std::path::Path;

/// Best-effort fsync of a file's parent directory, so an atomic
/// rename survives a real power cut.
///
/// Failure is logged, not fatal: the data itself is already written
/// and synced, and refusing to continue over an unsynced directory
/// entry would turn a small risk into a certain outage.
pub fn fsync_parent_dir(path: &Path) {
    let Some(parent) = path.parent() else { return };
    let dir = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };

    match File::open(dir) {
        Ok(handle) => {
            if let Err(e) = handle.sync_all() {
                tracing::warn!(dir = %dir.display(), error = %e, "could not fsync the directory");
            }
        }
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "could not open the directory to fsync it")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_with_no_parent_is_handled_rather_than_panicking() {
        // `Path::parent` of "tokens.json" is an empty path, not None;
        // opening "" fails, so this has to be turned into "." first.
        fsync_parent_dir(Path::new("tokens.json"));
    }

    #[test]
    fn a_missing_directory_is_a_warning_not_a_panic() {
        fsync_parent_dir(Path::new("/nonexistent-almanac-dir/file.json"));
    }

    #[test]
    fn syncing_a_real_directory_works() {
        let dir = std::env::temp_dir().join(format!("almanac-fsync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("thing");
        std::fs::write(&file, b"x").unwrap();
        fsync_parent_dir(&file);
        std::fs::remove_dir_all(&dir).ok();
    }
}

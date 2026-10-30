//! Deleting a profile file, by source id or by file name.

use std::path::{Path, PathBuf};

use super::paths::{PROFILE_EXT, profile_path, writable_remedy};
use crate::core::error::AlmanacError;

/// Deletes a source's profile (K21).
///
/// Really deletes it. The first version renamed the file aside and kept
/// the row on the page with a badge, copied from kyu at Kenny's
/// request; he then asked for the other thing — *"De optie retire die
/// we nu hebben moet de hele source wissen"* — and he is right that the
/// borrowed model did not fit. Kyu keeps a revoked app's row because
/// message history hangs off it. Here nothing hangs off a source: the
/// events it made belong to the calendar and stay there either way.
///
/// The events are deliberately left alone (Kenny, 2026-09-03): deleting
/// a source is a statement about the source, not about what already
/// happened. Removing months of calendar entries in one click is a
/// second, deliberate act, and not this one.
pub fn delete(dir: &Path, source_id: &str) -> Result<PathBuf, AlmanacError> {
    // Defence in depth: this argument arrives as a URL path segment,
    // and the file it names is about to be removed.
    if !crate::core::profile::source_id_is_safe(source_id) {
        return Err(AlmanacError::Config {
            message: format!("\"{source_id}\" is not a valid source_id"),
            remedy: "check the source id in the URL".to_string(),
        });
    }
    remove_existing(
        dir,
        profile_path(dir, source_id),
        "this source has no profile file — reload the page",
    )
}

/// Deletes an unusable profile by its file name (K23).
///
/// Separate from [`delete`] because a broken profile may have no
/// readable `source_id` to address it by — that is often exactly what
/// is wrong with it. The dashboard lists these by file name and deletes
/// them the same way.
pub fn delete_file(dir: &Path, file_name: &str) -> Result<PathBuf, AlmanacError> {
    if !is_profile_file_name(file_name) {
        return Err(AlmanacError::Config {
            message: format!("\"{file_name}\" is not a profile file name"),
            remedy: "check the name in the URL".to_string(),
        });
    }
    remove_existing(
        dir,
        dir.join(file_name),
        "reload the page — it may already be gone",
    )
}

/// The name arrives from a URL path segment. It must be one path
/// component of the shape this directory holds, and nothing else:
/// no separators, no traversal, no other extension.
fn is_profile_file_name(file_name: &str) -> bool {
    !file_name.is_empty()
        && !file_name.starts_with('.')
        && file_name.ends_with(&format!(".{PROFILE_EXT}"))
        && file_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Removes `path`, which must exist. `missing_remedy` is what to tell
/// the person when it does not.
fn remove_existing(
    dir: &Path,
    path: PathBuf,
    missing_remedy: &str,
) -> Result<PathBuf, AlmanacError> {
    if !path.exists() {
        return Err(AlmanacError::Config {
            message: format!("{} does not exist", path.display()),
            remedy: missing_remedy.to_string(),
        });
    }
    std::fs::remove_file(&path).map_err(|e| AlmanacError::Config {
        message: format!("failed to delete {}: {e}", path.display()),
        remedy: writable_remedy(dir),
    })?;
    Ok(path)
}

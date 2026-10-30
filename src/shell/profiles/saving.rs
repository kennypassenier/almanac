//! Saving a new profile submitted through the dashboard.

use std::io::Write;
use std::path::Path;

use super::loading::load_all;
use super::paths::{TMP_EXT, profile_path, writable_remedy};
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

/// Writes a new mapping profile submitted through the dashboard (K21)
/// and returns it.
///
/// Validation is `Profile::parse` — the same function startup uses —
/// rather than a second set of rules living in the browser. Two lists
/// of the same constraints drift, and the half that drifts is always
/// the one that says "fine" to something the service then refuses.
///
/// Refuses to overwrite: the dashboard adds sources, and an edit that
/// silently replaced a working profile would be indistinguishable from
/// a typo in the source_id.
pub fn save_new(dir: &Path, contents: &str) -> Result<Profile, AlmanacError> {
    let profile = Profile::parse(contents, "the submitted profile")?;
    ensure_dir(dir)?;
    ensure_unclaimed(dir, &profile)?;

    // feat-backup-1 (chassis-rs 3.1.0): waits out a backup pause before
    // writing a new profile file, so a nightly `tar` of the state root
    // never races this. Infrequent admin action, so the unbounded
    // `writing_blocking` is fine here (contrast the journal's hot path,
    // which uses the async, lock-held `writing`).
    let _ticket = chassis::shell::backup::writing_blocking();
    write_atomically(&profile_path(dir, &profile.source_id), contents)?;
    Ok(profile)
}

/// The directory may not exist yet on a fresh machine — creating it
/// here is what lets someone add their first source from the
/// dashboard instead of having to make a directory over ssh first.
fn ensure_dir(dir: &Path) -> Result<(), AlmanacError> {
    std::fs::create_dir_all(dir).map_err(|e| AlmanacError::Config {
        message: format!(
            "could not create the profiles directory {}: {e}",
            dir.display()
        ),
        remedy: "check the path is writable by the almanac user".to_string(),
    })
}

/// Refuses a source_id or file that is already taken.
///
/// Checked against what is on disk, not against the loaded set: a
/// profile added five seconds ago by another tab is on disk and not
/// yet in memory, and two profiles sharing a source_id stop the
/// service from starting at all (AR15).
fn ensure_unclaimed(dir: &Path, profile: &Profile) -> Result<(), AlmanacError> {
    let existing = load_all(dir);
    if existing
        .profiles
        .iter()
        .any(|p| p.source_id == profile.source_id)
    {
        return Err(AlmanacError::Config {
            message: format!(
                "a profile with source_id \"{}\" already exists",
                profile.source_id
            ),
            remedy: "choose a different source_id, or edit the existing profile on the machine"
                .to_string(),
        });
    }

    let path = profile_path(dir, &profile.source_id);
    if path.exists() {
        return Err(AlmanacError::Config {
            message: format!("{} already exists", path.display()),
            remedy: "choose a different source_id — the dashboard adds profiles, it does not replace them".to_string(),
        });
    }
    Ok(())
}

/// Write, flush, fsync, rename. A profile half-written by a crash would
/// stop the service from starting on its next boot — the same reason
/// the journal fsyncs before answering (AR16), applied to configuration.
fn write_atomically(path: &Path, contents: &str) -> Result<(), AlmanacError> {
    let tmp = path.with_extension(TMP_EXT);
    let failed = |what: &str, e: std::io::Error| AlmanacError::Config {
        message: format!("failed to {what} {}: {e}", path.display()),
        remedy: writable_remedy(path.parent().unwrap_or(Path::new("."))),
    };

    let mut file = std::fs::File::create(&tmp).map_err(|e| failed("create", e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| failed("write", e))?;
    file.sync_all().map_err(|e| failed("flush", e))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| failed("rename", e))?;
    Ok(())
}

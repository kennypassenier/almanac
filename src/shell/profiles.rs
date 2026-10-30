//! Reads mapping profiles off disk (I/O — AR13) and hands them to
//! `core::profile` for parsing and validation. `core::profile::Profile`
//! is the actual public type; this module only ever does file I/O.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::error::AlmanacError;
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

/// Loads every `*.toml` profile in `dir`. **Never fails.**
///
/// Kenny, 2026-09-03: *"een kapot profiel mag niet het opstarten van de
/// app belemmeren … De app moet ten allen tijde zelf kunnen opstarten
/// op zichzelf. Dingen buiten de app mogen dat niet beïnvloeden."*
///
/// That is a stronger rule than the one this module used to follow, and
/// a better one. A profile is a file outside the program: it can be
/// half-written by an editor, left behind by an older version, or
/// duplicated by a copy-paste. Any of those used to stop the whole
/// service — including the dashboard, which is the one place from which
/// they could have been fixed. A service that cannot start because of a
/// file it is supposed to manage has no way back.
///
/// So every per-file problem yields an `Unusable` entry instead: the
/// source is not served, it is reported at startup, and it is listed on
/// the dashboard with a delete button. A missing or unreadable
/// directory is the same — zero profiles, one report, still serving.
pub fn load_all(dir: &Path) -> Loaded {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            return Loaded {
                profiles: Vec::new(),
                unusable: vec![Unusable {
                    path: dir.to_path_buf(),
                    reason: format!(
                        "the profiles directory could not be read: {e} — create it, or add a \
                         source from the dashboard and almanac will"
                    ),
                }],
            };
        }
    };

    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    // Deterministic order, which now decides more than error text: when
    // two files claim the same source_id, the first one sorted wins and
    // the other is reported. Deterministic beats "whichever the
    // filesystem handed over first".
    paths.sort();

    let mut profiles: Vec<Profile> = Vec::with_capacity(paths.len());
    let mut unusable = Vec::new();

    for path in paths {
        let origin = path.display().to_string();

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) => {
                unusable.push(Unusable {
                    path,
                    reason: format!("could not be read: {e}"),
                });
                continue;
            }
        };

        if let Some(schema_version) = crate::core::profile::outdated_version(&contents) {
            unusable.push(Unusable {
                path,
                reason: format!(
                    "written for schema_version {schema_version}; this build reads {} — reduce \
                     it to source_id and target_calendar_id, and have the source send almanac's \
                     event shape",
                    crate::core::profile::SUPPORTED_SCHEMA_VERSION
                ),
            });
            continue;
        }

        let profile = match Profile::parse(&contents, &origin) {
            Ok(profile) => profile,
            Err(e) => {
                unusable.push(Unusable {
                    path,
                    reason: e.to_string(),
                });
                continue;
            }
        };

        // AR15: two profiles sharing a source_id make the upsert key
        // ambiguous. The first one wins and the second is reported,
        // rather than both being lost along with every other source.
        if let Some(clash) = profiles.iter().find(|p| p.source_id == profile.source_id) {
            unusable.push(Unusable {
                path,
                reason: format!(
                    "source_id \"{}\" is already used by another profile, which is being served \
                     instead — a source_id is an identity (AR15) and two files cannot share one",
                    clash.source_id
                ),
            });
            continue;
        }

        profiles.push(profile);
    }

    Loaded { profiles, unusable }
}

/// The same set, keyed by `source_id` — the shape the running service
/// actually reads. Built here rather than at each call site so that
/// startup and a K21 reload cannot key the map differently.
pub fn load_map(dir: &Path) -> HashMap<String, Profile> {
    load_all(dir)
        .profiles
        .into_iter()
        .map(|p| (p.source_id.clone(), p))
        .collect()
}

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

    // Checked against what is on disk, not against the loaded set: a
    // profile added five seconds ago by another tab is on disk and not
    // yet in memory, and two profiles sharing a source_id stop the
    // service from starting at all (AR15).
    // The directory may not exist yet on a fresh machine — creating it
    // here is what lets someone add their first source from the
    // dashboard instead of having to make a directory over ssh first.
    std::fs::create_dir_all(dir).map_err(|e| AlmanacError::Config {
        message: format!(
            "could not create the profiles directory {}: {e}",
            dir.display()
        ),
        remedy: "check the path is writable by the almanac user".to_string(),
    })?;

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

    // feat-backup-1 (chassis-rs 3.1.0): waits out a backup pause before
    // writing a new profile file, so a nightly `tar` of the state root
    // never races this. Infrequent admin action, so the unbounded
    // `writing_blocking` is fine here (contrast the journal's hot path,
    // which uses the async, lock-held `writing`).
    let _ticket = chassis::shell::backup::writing_blocking();
    write_atomically(&path, contents)?;
    Ok(profile)
}

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

    let path = profile_path(dir, source_id);
    if !path.exists() {
        return Err(AlmanacError::Config {
            message: format!("{} does not exist", path.display()),
            remedy: "this source has no profile file — reload the page".to_string(),
        });
    }

    std::fs::remove_file(&path).map_err(|e| AlmanacError::Config {
        message: format!("failed to delete {}: {e}", path.display()),
        remedy: format!(
            "check the profiles directory {} is writable by the almanac user",
            dir.display()
        ),
    })?;
    Ok(path)
}

/// Deletes an unusable profile by its file name (K23).
///
/// Separate from [`delete`] because a broken profile may have no
/// readable `source_id` to address it by — that is often exactly what
/// is wrong with it. The dashboard lists these by file name and deletes
/// them the same way.
pub fn delete_file(dir: &Path, file_name: &str) -> Result<PathBuf, AlmanacError> {
    // The name arrives from a URL path segment. It must be one path
    // component of the shape this directory holds, and nothing else:
    // no separators, no traversal, no other extension.
    let looks_safe = !file_name.is_empty()
        && !file_name.starts_with('.')
        && file_name.ends_with(".toml")
        && file_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !looks_safe {
        return Err(AlmanacError::Config {
            message: format!("\"{file_name}\" is not a profile file name"),
            remedy: "check the name in the URL".to_string(),
        });
    }

    let path = dir.join(file_name);
    if !path.exists() {
        return Err(AlmanacError::Config {
            message: format!("{} does not exist", path.display()),
            remedy: "reload the page — it may already be gone".to_string(),
        });
    }

    std::fs::remove_file(&path).map_err(|e| AlmanacError::Config {
        message: format!("failed to delete {}: {e}", path.display()),
        remedy: format!(
            "check the profiles directory {} is writable by the almanac user",
            dir.display()
        ),
    })?;
    Ok(path)
}

/// `<dir>/<source_id>.toml`. Safe because `Profile::parse` has already
/// refused any source_id that is not filename-shaped
/// (`core::profile::source_id_is_safe`); this function is never reached
/// with an unvalidated id.
fn profile_path(dir: &Path, source_id: &str) -> PathBuf {
    dir.join(format!("{source_id}.toml"))
}

/// Write, flush, fsync, rename. A profile half-written by a crash would
/// stop the service from starting on its next boot — the same reason
/// the journal fsyncs before answering (AR16), applied to configuration.
fn write_atomically(path: &Path, contents: &str) -> Result<(), AlmanacError> {
    let tmp = path.with_extension("toml.tmp");
    let failed = |what: &str, e: std::io::Error| AlmanacError::Config {
        message: format!("failed to {what} {}: {e}", path.display()),
        remedy: format!(
            "check the profiles directory {} is writable by the almanac user",
            path.parent().unwrap_or(Path::new(".")).display()
        ),
    };

    let mut file = std::fs::File::create(&tmp).map_err(|e| failed("create", e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| failed("write", e))?;
    file.sync_all().map_err(|e| failed("flush", e))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| failed("rename", e))?;
    Ok(())
}

#[cfg(test)]
mod tests;

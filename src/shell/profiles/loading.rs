//! Loading every profile in a directory. **Never fails.**

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::paths::PROFILE_EXT;
use super::types::{Loaded, Unusable};
use crate::core::profile::{self, Profile};

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
    let (paths, mut unusable) = match profile_paths(dir) {
        Ok(found) => found,
        Err(unreadable) => {
            return Loaded {
                profiles: Vec::new(),
                unusable: vec![unreadable],
            };
        }
    };

    let mut profiles: Vec<Profile> = Vec::with_capacity(paths.len());
    for path in paths {
        match load_one(&path, &profiles) {
            Ok(profile) => profiles.push(profile),
            Err(reason) => unusable.push(Unusable { path, reason }),
        }
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

/// The sorted `*.toml` paths in `dir`, plus an `Unusable` for each
/// directory entry that could not even be read. A directory that cannot
/// be listed at all is the `Err`.
fn profile_paths(dir: &Path) -> Result<(Vec<PathBuf>, Vec<Unusable>), Unusable> {
    let entries = std::fs::read_dir(dir).map_err(|e| Unusable {
        path: dir.to_path_buf(),
        reason: format!(
            "the profiles directory could not be read: {e} — create it, or add a \
             source from the dashboard and almanac will"
        ),
    })?;

    let mut paths = Vec::new();
    let mut unusable = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(e) => {
                tracing::warn!(error = %e, dir = %dir.display(), "a profiles directory entry could not be read");
                unusable.push(Unusable {
                    path: dir.to_path_buf(),
                    reason: format!("an entry of the profiles directory could not be read: {e}"),
                });
            }
        }
    }
    paths.retain(|path| path.extension().is_some_and(|ext| ext == PROFILE_EXT));
    // Deterministic order, which now decides more than error text: when
    // two files claim the same source_id, the first one sorted wins and
    // the other is reported. Deterministic beats "whichever the
    // filesystem handed over first".
    paths.sort();
    Ok((paths, unusable))
}

/// Reads and validates one file; `Err` is the sentence for its `Unusable`.
/// `served` holds the profiles accepted so far, for the duplicate check.
fn load_one(path: &Path, served: &[Profile]) -> Result<Profile, String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("could not be read: {e}"))?;

    if let Some(schema_version) = profile::outdated_version(&contents) {
        return Err(format!(
            "written for schema_version {schema_version}; this build reads {} — reduce \
             it to source_id and target_calendar_id, and have the source send almanac's \
             event shape",
            profile::SUPPORTED_SCHEMA_VERSION
        ));
    }

    let origin = path.display().to_string();
    let profile = Profile::parse(&contents, &origin).map_err(|e| e.to_string())?;

    // AR15: two profiles sharing a source_id make the upsert key
    // ambiguous. The first one wins and the second is reported,
    // rather than both being lost along with every other source.
    if let Some(clash) = served.iter().find(|p| p.source_id == profile.source_id) {
        return Err(format!(
            "source_id \"{}\" is already used by another profile, which is being served \
             instead — a source_id is an identity (AR15) and two files cannot share one",
            clash.source_id
        ));
    }
    Ok(profile)
}

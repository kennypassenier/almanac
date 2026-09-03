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
mod tests {
    use super::*;
    use std::io::Write;

    fn write_profile(dir: &Path, filename: &str, source_id: &str) {
        let toml = format!(
            r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"

"#
        );
        let mut file = std::fs::File::create(dir.join(filename)).unwrap();
        file.write_all(toml.as_bytes()).unwrap();
    }

    #[test]
    fn loads_every_toml_file_in_the_directory() {
        let dir =
            std::env::temp_dir().join(format!("almanac-profiles-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_profile(&dir, "a.toml", "home-assistant");
        write_profile(&dir, "b.toml", "uptime-kuma");
        std::fs::write(dir.join("not-a-profile.txt"), "ignored").unwrap();

        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 2);
        assert!(loaded.unusable.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k23_a_profile_from_an_older_format_is_skipped_and_the_rest_still_load() {
        // Kenny, 2026-09-03: "hij mag wel niet weigeren om op te
        // starten met profielen van een oude versie". Taking every
        // other source down over one outdated file is the wrong blast
        // radius — it is not a mistake, it is a known state.
        let dir = temp_dir("outdated");
        write_profile(&dir, "new.toml", "home-assistant");
        std::fs::write(
            dir.join("old.toml"),
            "schema_version = 1\nsource_id = \"uptime-kuma\"\n             target_calendar_id = \"infra\"\n\n[mapping]\ntitle_field = \"monitor.name\"\n             start_field = \"time\"\nduration_minutes = 15\n",
        )
        .unwrap();

        let loaded = load_all(&dir);

        assert_eq!(loaded.profiles.len(), 1, "the usable profile must load");
        assert_eq!(loaded.profiles[0].source_id, "home-assistant");
        assert_eq!(loaded.unusable.len(), 1, "and the old one must be reported");
        assert!(loaded.unusable[0].reason.contains("schema_version 1"));
        assert_eq!(loaded.unusable[0].file_name(), "old.toml");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_directory_names_itself_with_a_remedy() {
        let dir = std::env::temp_dir().join("almanac-profiles-definitely-does-not-exist");
        let loaded = load_all(&dir);
        assert!(loaded.profiles.is_empty());
        assert_eq!(loaded.unusable.len(), 1);
        assert!(
            loaded.unusable[0]
                .path
                .display()
                .to_string()
                .contains(&dir.display().to_string())
        );
        assert!(!loaded.unusable[0].reason.is_empty(), "it must say why");
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "almanac-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SUBMITTED: &str = r#"
schema_version = 2
source_id = "kobo"
target_calendar_id = "primary"

"#;

    #[test]
    fn k21_a_saved_profile_can_be_read_back_by_the_loader() {
        // The round trip is the whole promise: what the dashboard
        // writes must be what startup would have accepted.
        let dir = temp_dir("save");
        let saved = save_new(&dir, SUBMITTED).unwrap();
        assert_eq!(saved.source_id, "kobo");

        let map = load_map(&dir);
        assert!(map.contains_key("kobo"));
        assert!(dir.join("kobo.toml").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_an_invalid_profile_writes_nothing_at_all() {
        // A half-written profile stops the NEXT start, long after
        // whoever submitted it has closed the tab.
        let dir = temp_dir("invalid");
        let err = save_new(&dir, "this is not toml at all").unwrap_err();
        assert!(err.to_string().contains("the submitted profile"));
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "a rejected profile must leave the directory untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_a_duplicate_source_id_is_refused_before_it_can_break_startup() {
        // Two profiles sharing a source_id do not degrade the service,
        // they stop it from starting (AR15) — so this has to be caught
        // here rather than discovered at the next restart.
        let dir = temp_dir("dup");
        save_new(&dir, SUBMITTED).unwrap();
        let err = save_new(&dir, SUBMITTED).unwrap_err();
        assert!(err.to_string().contains("kobo"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_a_hostile_source_id_cannot_write_outside_the_profiles_directory() {
        // The file name comes from the source_id, and the source_id
        // comes from a browser. Rejected in core::profile, asserted
        // here because this is the layer that would do the damage.
        let dir = temp_dir("escape");
        let hostile = SUBMITTED.replace("\"kobo\"", "\"../escaped\"");
        let err = save_new(&dir, &hostile).unwrap_err();
        assert!(err.to_string().contains("source_id"));
        assert!(
            !dir.parent().unwrap().join("escaped.toml").exists(),
            "nothing may be written outside the profiles directory"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_an_existing_file_is_never_overwritten() {
        // This page adds sources. Silently replacing a working profile
        // because a source_id was retyped is the one mistake that
        // cannot be undone from the dashboard.
        let dir = temp_dir("overwrite");
        std::fs::write(dir.join("kobo.toml"), "schema_version = 1\n").unwrap();
        let err = save_new(&dir, SUBMITTED).unwrap_err();
        assert!(err.to_string().contains("kobo.toml"));
        assert_eq!(
            std::fs::read_to_string(dir.join("kobo.toml")).unwrap(),
            "schema_version = 1\n",
            "the existing file must be byte-identical afterwards"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_no_temporary_file_is_left_behind_by_a_successful_save() {
        // The atomic write uses a .toml.tmp beside the target; a
        // leftover would be loaded as a profile on the next start only
        // if the extension filter ever loosened, and would look like a
        // duplicate when it did.
        let dir = temp_dir("tmp");
        save_new(&dir, SUBMITTED).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["kobo.toml".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_a_deleted_profile_is_gone_from_the_disk_and_the_loaded_set() {
        // Kenny asked for the whole source to go, not to be set aside:
        // "De optie retire die we nu hebben moet de hele source wissen".
        let dir = temp_dir("delete");
        save_new(&dir, SUBMITTED).unwrap();

        let removed = delete(&dir, "kobo").unwrap();

        assert!(!removed.exists(), "the profile file must be gone");
        assert!(
            !load_map(&dir).contains_key("kobo"),
            "a deleted source must not be loaded"
        );
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "nothing may be left behind, not even a renamed copy"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_deleting_something_that_is_not_there_says_so() {
        let dir = temp_dir("delete-missing");
        let err = delete(&dir, "kobo").unwrap_err();
        assert!(err.to_string().contains("kobo.toml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_a_hostile_source_id_cannot_delete_a_file_outside_the_directory() {
        // Unlike save_new, this one takes its id straight from a URL
        // path segment — nothing has parsed a profile on the way in,
        // and the file it names is about to be removed.
        let dir = temp_dir("delete-escape");
        let err = delete(&dir, "../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("source_id"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_profiles_shipped_in_this_repository_load() {
        // The runbook tells you to copy these onto the machine during
        // the first install. Nothing loaded them in a test, so a
        // profile could rot — or the directory could be named
        // something else entirely, which is exactly what the runbook
        // got wrong — and the first thing to notice would be a
        // half-provisioned LXC refusing to start.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/profiles");
        let loaded = load_all(&dir);
        assert!(
            loaded.unusable.is_empty(),
            "the shipped profiles must all be usable, got {:?}",
            loaded.unusable
        );
        let profiles = loaded.profiles;

        assert!(
            profiles.len() >= 2,
            "expected the home-assistant and everything profiles, got {}",
            profiles.len()
        );
        for expected in ["home-assistant", "everything"] {
            assert!(
                profiles.iter().any(|p| p.source_id == expected),
                "the {expected} profile is missing from the shipped set"
            );
        }
    }
}

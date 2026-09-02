//! Reads mapping profiles off disk (I/O — AR13) and hands them to
//! `core::profile` for parsing and validation. `core::profile::Profile`
//! is the actual public type; this module only ever does file I/O.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::error::AlmanacError;
use crate::core::profile::{Profile, validate_unique_source_ids};

/// Loads and validates every `*.toml` profile in `dir`. Fails on the
/// first unreadable or invalid file, and on any duplicate `source_id`
/// across the whole set (AR15) — a broken profile must stop startup
/// with a message naming the file, not silently skip it (standing
/// rule 12: no silent fallbacks).
pub fn load_all(dir: &Path) -> Result<Vec<Profile>, AlmanacError> {
    let entries = std::fs::read_dir(dir).map_err(|e| AlmanacError::Config {
        message: format!("failed to read profiles directory {}: {e}", dir.display()),
        remedy: format!(
            "create {} with at least one *.toml mapping profile",
            dir.display()
        ),
    })?;

    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    // Deterministic order so "index 0 and 1" in a duplicate-source_id
    // error means the same thing on every run.
    paths.sort();

    let mut profiles = Vec::with_capacity(paths.len());
    for path in paths {
        let origin = path.display().to_string();
        let contents = std::fs::read_to_string(&path).map_err(|e| AlmanacError::Config {
            message: format!("failed to read {origin}: {e}"),
            remedy: format!("check {origin} exists and is readable"),
        })?;
        profiles.push(Profile::parse(&contents, &origin)?);
    }

    validate_unique_source_ids(&profiles)?;

    Ok(profiles)
}

/// The same set, keyed by `source_id` — the shape the running service
/// actually reads. Built here rather than at each call site so that
/// startup and a K21 reload cannot key the map differently.
pub fn load_map(dir: &Path) -> Result<HashMap<String, Profile>, AlmanacError> {
    Ok(load_all(dir)?
        .into_iter()
        .map(|p| (p.source_id.clone(), p))
        .collect())
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
    let existing = load_all(dir)?;
    if existing.iter().any(|p| p.source_id == profile.source_id) {
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

/// What a retired profile's file is renamed to.
///
/// Deliberately not a deletion, and deliberately not `.toml`: the
/// loader only reads `*.toml`, so a retired source disappears from the
/// running set while the file — and therefore the record that this
/// source ever existed, and everything it was configured to do — stays
/// where it was. Copied from kyu, where revoking an app keeps its row
/// rather than erasing it (Kenny, 2026-09-02: "kopieer het kyu model").
const RETIRED_SUFFIX: &str = ".toml.retired";

/// Retires a source's profile (K21): renames it out of the loaded set
/// and returns where it went.
///
/// Reversible by hand — rename the file back and reload — which is the
/// property that makes a button safe to press. Deleting outright would
/// need a confirmation dialog to be honest about itself; this does not.
pub fn retire(dir: &Path, source_id: &str) -> Result<PathBuf, AlmanacError> {
    // Defence in depth: this argument arrives as a URL path segment,
    // and the profiles it can reach are named after it.
    if !crate::core::profile::source_id_is_safe(source_id) {
        return Err(AlmanacError::Config {
            message: format!("\"{source_id}\" is not a valid source_id"),
            remedy: "check the source id in the URL".to_string(),
        });
    }

    let from = profile_path(dir, source_id);
    if !from.exists() {
        return Err(AlmanacError::Config {
            message: format!("{} does not exist", from.display()),
            remedy: "this source has no profile file to retire — reload the page".to_string(),
        });
    }

    // A source_id can be retired, recreated and retired again. Each
    // retirement keeps its own file rather than overwriting the last,
    // because the older one is the record of a different configuration.
    let mut to = dir.join(format!("{source_id}{RETIRED_SUFFIX}"));
    let mut n = 2;
    while to.exists() {
        to = dir.join(format!("{source_id}{RETIRED_SUFFIX}-{n}"));
        n += 1;
    }

    std::fs::rename(&from, &to).map_err(|e| AlmanacError::Config {
        message: format!("failed to retire {}: {e}", from.display()),
        remedy: format!(
            "check the profiles directory {} is writable by the almanac user",
            dir.display()
        ),
    })?;
    Ok(to)
}

/// The source ids whose profiles have been retired, sorted.
///
/// Read from the directory rather than remembered in memory: the
/// record has to survive a restart, and the file is the record.
pub fn retired(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.split_once(RETIRED_SUFFIX)?.0.to_string();
            (!id.is_empty()).then_some(id)
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
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
schema_version = 1
source_id = "{source_id}"
target_calendar_id = "primary"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
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

        let profiles = load_all(&dir).unwrap();
        assert_eq!(profiles.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_directory_names_itself_with_a_remedy() {
        let dir = std::env::temp_dir().join("almanac-profiles-definitely-does-not-exist");
        let err = load_all(&dir).unwrap_err();
        assert!(err.to_string().contains(&dir.display().to_string()));
    }

    #[test]
    fn duplicate_source_ids_across_files_are_rejected() {
        let dir =
            std::env::temp_dir().join(format!("almanac-profiles-dup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_profile(&dir, "a.toml", "same-id");
        write_profile(&dir, "b.toml", "same-id");

        let err = load_all(&dir).unwrap_err();
        assert!(err.to_string().contains("same-id"));

        std::fs::remove_dir_all(&dir).ok();
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
schema_version = 1
source_id = "kobo"
target_calendar_id = "primary"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
"#;

    #[test]
    fn k21_a_saved_profile_can_be_read_back_by_the_loader() {
        // The round trip is the whole promise: what the dashboard
        // writes must be what startup would have accepted.
        let dir = temp_dir("save");
        let saved = save_new(&dir, SUBMITTED).unwrap();
        assert_eq!(saved.source_id, "kobo");

        let map = load_map(&dir).unwrap();
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
    fn k21_a_retired_profile_leaves_the_loaded_set_but_not_the_disk() {
        // The kyu model: ending a source keeps the record that it
        // existed. A file that is gone answers no questions later.
        let dir = temp_dir("retire");
        save_new(&dir, SUBMITTED).unwrap();

        let kept = retire(&dir, "kobo").unwrap();

        assert!(kept.exists(), "the profile must survive as a file");
        assert!(!dir.join("kobo.toml").exists());
        assert!(
            !load_map(&dir).unwrap().contains_key("kobo"),
            "a retired source must not be loaded"
        );
        assert_eq!(retired(&dir), vec!["kobo".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_retiring_the_same_source_twice_keeps_both_records() {
        // A source_id can be retired, recreated with different settings
        // and retired again; the first file is the record of the first
        // configuration and overwriting it would lose exactly the
        // history this feature exists to keep.
        let dir = temp_dir("retire-twice");
        save_new(&dir, SUBMITTED).unwrap();
        let first = retire(&dir, "kobo").unwrap();
        save_new(&dir, SUBMITTED).unwrap();
        let second = retire(&dir, "kobo").unwrap();

        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
        assert_eq!(retired(&dir), vec!["kobo".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_retiring_something_that_is_not_there_says_so() {
        let dir = temp_dir("retire-missing");
        let err = retire(&dir, "kobo").unwrap_err();
        assert!(err.to_string().contains("kobo.toml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn k21_a_hostile_source_id_cannot_rename_a_file_outside_the_directory() {
        // Unlike save_new, this one takes its id straight from a URL
        // path segment — nothing has parsed a profile on the way in.
        let dir = temp_dir("retire-escape");
        let err = retire(&dir, "../../etc/passwd").unwrap_err();
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
        let profiles = load_all(&dir).expect("the shipped profiles must parse");

        assert!(
            profiles.len() >= 3,
            "expected the home-assistant, grafana and uptime-kuma profiles, got {}",
            profiles.len()
        );
        for expected in ["home-assistant", "grafana", "uptime-kuma"] {
            assert!(
                profiles.iter().any(|p| p.source_id == expected),
                "the {expected} profile is missing from the shipped set"
            );
        }
    }
}

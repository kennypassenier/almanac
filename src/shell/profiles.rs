//! Reads mapping profiles off disk (I/O — AR13) and hands them to
//! `core::profile` for parsing and validation. `core::profile::Profile`
//! is the actual public type; this module only ever does file I/O.

use std::path::Path;

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

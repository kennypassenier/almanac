//! Loading profiles from a directory, skipped files and the shipped set.

use super::*;

#[test]
fn loads_every_toml_file_in_the_directory() {
    let dir = std::env::temp_dir().join(format!("almanac-profiles-test-{}", std::process::id()));
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

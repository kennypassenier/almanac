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

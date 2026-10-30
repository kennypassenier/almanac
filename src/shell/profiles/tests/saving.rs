//! Saving a submitted profile (K21): validation, duplicates, escapes.

use super::*;

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

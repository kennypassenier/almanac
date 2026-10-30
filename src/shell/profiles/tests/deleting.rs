//! Deleting a saved profile (K21).

use super::*;

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

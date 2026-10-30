use super::*;

#[test]
fn buckets_count_the_last_hours_oldest_first() {
    let now = 10 * 3600 + 120;
    let v = hourly_buckets(&[now, now - 60, 9 * 3600, 2 * 3600, 0], now, 3);
    assert_eq!(v, vec![0, 1, 2]);
}

#[test]
fn a_reason_splits_into_why_and_what_now_without_a_path() {
    let dir = std::path::Path::new("/srv/profiles");
    let (why, remedy) = split_reason(
        "written for schema_version 1; this build reads 2 — reduce it to two fields",
        dir,
        "old.toml",
    );
    assert_eq!(
        why,
        "Old.toml: written for schema_version 1; this build reads 2."
    );
    assert_eq!(remedy, "Reduce it to two fields. Or delete the file.");
    let (_, plain) = split_reason("could not be read", dir, "x.toml");
    assert!(plain.contains("reload the profiles"));
}

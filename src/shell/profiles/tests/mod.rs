mod deleting;
mod loading;
mod saving;

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

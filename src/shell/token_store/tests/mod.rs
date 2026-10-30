mod basics;
mod concurrency;
mod key_check;
mod sessions;

use super::*;

fn temp_store(name: &str) -> (TokenStore, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "almanac-tokenstore-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tokens.json");
    (TokenStore::with_key(path, [3u8; KEY_BYTES]), dir)
}

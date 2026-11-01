use super::FILES;
use std::path::Path;

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("web/ is readable") {
        let path = entry.expect("a web/ entry").path();
        if path.is_dir() {
            walk(&path, root, out);
        } else {
            let rel = path.strip_prefix(root).expect("under web/");
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// A file on disk that is not in FILES is a 404 in the binary: the
/// Deliveries page once shipped without its actions and groups scripts.
#[test]
fn every_web_file_is_embedded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    let mut on_disk = Vec::new();
    walk(&root, &root, &mut on_disk);
    let missing: Vec<_> = on_disk
        .iter()
        .filter(|f| !FILES.iter().any(|(name, _)| name == f))
        .collect();
    assert!(missing.is_empty(), "not embedded in FILES: {missing:?}");
}

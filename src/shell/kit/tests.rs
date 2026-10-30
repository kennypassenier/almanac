use super::*;

#[tokio::test]
async fn tokens_are_not_imported_once_the_kit_holds_the_clients() {
    let dir = std::env::temp_dir().join(format!("almanac-kit-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let tokens = dir.join("tokens.json");
    std::fs::write(&tokens, b"{}").unwrap();
    // The key is deliberately invalid: reaching it would be an error, so
    // Ok(0) proves the early return and that no clients.json.enc is made.
    std::fs::write(dir.join("almanac-kit.db"), b"").unwrap();
    assert_eq!(import_source_tokens(&dir, &tokens, "").await, Ok(0));
    std::fs::remove_file(dir.join("almanac-kit.db")).unwrap();
    std::fs::write(dir.join("clients.json.enc.migrated"), b"").unwrap();
    assert_eq!(import_source_tokens(&dir, &tokens, "").await, Ok(0));
    assert!(!dir.join("clients.json.enc").exists());
    std::fs::remove_dir_all(&dir).ok();
}

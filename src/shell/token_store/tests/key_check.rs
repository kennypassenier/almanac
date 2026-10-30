//! The startup key check and keys that do not match the file.

use super::*;

#[tokio::test]
async fn an_empty_store_passes_the_startup_key_check() {
    let (store, dir) = temp_store("keycheck-empty");
    assert!(store.verify_key_opens_store().await.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_startup_key_check_passes_with_the_right_key() {
    let (store, dir) = temp_store("keycheck-ok");
    store.issue("home-assistant", "tok", "now").await.unwrap();
    assert!(store.verify_key_opens_store().await.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_startup_key_check_catches_a_wrong_key_instead_of_401ing_later() {
    // The failure this exists to prevent: without the check the
    // process starts happily and every source gets a 401 against a
    // store that looks perfectly intact.
    let (store, dir) = temp_store("keycheck-wrong");
    store.issue("home-assistant", "tok", "now").await.unwrap();

    let wrong = TokenStore::with_key_loading(store.path().to_path_buf(), [9u8; KEY_BYTES])
        .expect("the file itself parses fine — that is the whole problem");
    let err = wrong.verify_key_opens_store().await.unwrap_err();

    assert!(err.to_string().contains("does not open"));
    assert!(
        err.remedy().contains("escrow"),
        "the remedy must point at the way out"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_store_written_with_one_key_does_not_verify_under_another() {
    // Why the encryption key must stay stable, demonstrated: a
    // changed key does not silently accept or reject at random, it
    // consistently fails to match.
    let (store, dir) = temp_store("keychange");
    store
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();

    let reopened = TokenStore::with_key(store.path().to_path_buf(), [4u8; KEY_BYTES]);
    {
        let mut tokens = reopened.tokens.write().await;
        *tokens = store.tokens.read().await.clone();
    }
    assert!(!reopened.verify("home-assistant", "tok-abc").await);

    std::fs::remove_dir_all(&dir).ok();
}

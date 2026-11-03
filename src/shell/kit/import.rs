//! The one-time move of the 3.x per-source tokens into the kit's clients.

use std::path::Path;

use chassis::core::clients::{Client, ClientsFile};
use chassis::shell::store::{ClientStore, EncryptedFile, FileClientStore};

use crate::shell::token_store::TokenStore;

/// The sealed client file the kit read before chassis-rs 3.11.0.
const KIT_CLIENTS_FILE: &str = "clients.json.enc";
/// What the kit renames that file to once it moved it into its database.
const KIT_CLIENTS_MIGRATED: &str = "clients.json.enc.migrated";

/// A2-1 · one-time import of the 3.x per-source tokens into the kit's
/// client store, so JobTracker and every other source keep their tokens.
///
/// Runs before the kit opens the store; does nothing once the kit holds
/// the clients or when there is no 3.x store to read, so it is
/// idempotent across restarts. The tokens are copied unchanged.
pub async fn import_source_tokens(
    state_dir: &Path,
    token_store: &Path,
    key_hex: &str,
) -> Result<usize, String> {
    if already_imported(state_dir) || !token_store.exists() {
        return Ok(0);
    }
    let key_bytes: [u8; 32] = hex::decode(key_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "ALMANAC_SECRET_KEY must be 64 hex characters".to_string())?;
    let store = TokenStore::with_key_loading(token_store.to_path_buf(), key_bytes)
        .map_err(|e| format!("{e}: {}", e.remedy()))?;
    let clients = read_3x_tokens(&store).await?;
    if clients.is_empty() {
        return Ok(0);
    }
    write_kit_clients(&state_dir.join(KIT_CLIENTS_FILE), key_hex, &clients)?;
    Ok(clients.len())
}

/// Since chassis-rs 3.11.0 the kit's clients live in its database and
/// the old sealed file is renamed. Any of the three present means the
/// clients are already the kit's: writing a fresh sealed file now would
/// make the kit migrate it a second time and trip over the duplicate ids.
fn already_imported(state_dir: &Path) -> bool {
    let kit_db = chassis::storage::kit_file_name("almanac");
    [KIT_CLIENTS_FILE, KIT_CLIENTS_MIGRATED, kit_db.as_str()]
        .iter()
        .any(|name| state_dir.join(name).exists())
}

/// Every source in the 3.x store with its token, as a kit client.
async fn read_3x_tokens(store: &TokenStore) -> Result<Vec<Client>, String> {
    let mut clients = Vec::new();
    for (source_id, issued_at) in store.list().await {
        let Some(token) = store.reveal(&source_id).await.map_err(|e| e.to_string())? else {
            tracing::warn!(
                source_id,
                "the 3.x token store lists this source without a token; it is not imported"
            );
            continue;
        };
        // chassis-rs 2.0.0: Client is #[non_exhaustive], so a struct
        // literal from outside the kit no longer compiles — build with
        // Client::adopted instead, which fills every field the kit adds
        // later with its own default.
        clients.push(Client::adopted(
            format!("source-{source_id}"),
            source_id,
            token,
            issued_at,
        ));
    }
    Ok(clients)
}

/// Seals `clients` into the kit's client file at `file`.
fn write_kit_clients(file: &Path, key_hex: &str, clients: &[Client]) -> Result<(), String> {
    let kit_key = chassis::core::crypto::Key::parse_hex("ALMANAC_SECRET_KEY", key_hex, key_hex)
        .map_err(|e| e.to_string())?;
    let kit_store =
        FileClientStore::open(EncryptedFile::new(file.to_path_buf(), kit_key, "clients"))
            .map_err(|e| e.to_string())?;
    kit_store
        .update(&mut |clients_file: &mut ClientsFile| {
            clients_file.clients.extend(clients.iter().cloned());
            Ok(clients.last().cloned().expect("at least one source"))
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

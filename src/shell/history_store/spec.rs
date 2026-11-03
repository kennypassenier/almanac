//! The schema, the app database spec and the Storage page's data kind.

use chassis::storage::{AppDbSpec, DataKind, Filter, ImportContext, Migration, TimeFormat};

use super::import::import_from;
use crate::core::history;

/// The schema, append only. Counters per day are counted from the rows
/// (`day` is indexed), so a clean-up on the Storage page keeps the heatmap
/// true and there is no second table to drift.
pub fn migrations() -> Vec<Migration> {
    vec![Migration::sql(
        "CREATE TABLE deliveries (
            id TEXT PRIMARY KEY,
            delivered_at INTEGER NOT NULL,
            day TEXT NOT NULL,
            source TEXT NOT NULL,
            calendar TEXT NOT NULL,
            title TEXT NOT NULL,
            result TEXT NOT NULL CHECK (result IN ('created', 'updated', 'failed')),
            reason TEXT,
            upsert_key TEXT
        ) STRICT;
        CREATE INDEX deliveries_day ON deliveries (day);
        CREATE INDEX deliveries_upsert_key ON deliveries (upsert_key, delivered_at);
        CREATE INDEX deliveries_delivered_at ON deliveries (delivered_at);",
    )]
}

/// The app database spec: the schema (the delivery history, then the
/// journal, which imports `journal_file` once) and the one-time history import.
pub fn app_db_spec(journal_file: std::path::PathBuf) -> AppDbSpec {
    let mut steps = migrations();
    steps.push(crate::shell::journal::migrate::migration(journal_file));
    // A copy of almanac.db before every schema step (the journal import is
    // one), next to the journal file's own .pre-db copy; three are kept.
    AppDbSpec::new(steps)
        .with_options(chassis::storage::DbOptions::new().pre_migration_snapshots(3))
        .with_import(|tx, ctx: &ImportContext| import_from(tx, ctx))
}

/// The Storage page's "Delivery log". Rows older than [`history::MAX_DAYS`] days
/// are finished by their own definition and pruned daily by the kit;
/// anything else goes through the page's preview and typed confirmation.
pub fn kind() -> DataKind {
    DataKind::new("deliveries", "Delivery log")
        .description(&format!(
            "Every event Almanac finished with, delivered or set aside as failed: \
             what the History page shows. Rows older than {} days are removed on their own.",
            history::MAX_DAYS
        ))
        .unit("deliveries")
        .table("deliveries")
        .time_column("delivered_at", TimeFormat::UnixSeconds)
        .filter(Filter::choice(
            "result",
            "Result",
            "result",
            ["created", "updated", "failed"],
        ))
        .filter(Filter::text("source", "Source", "source"))
        .finished_when(format!(
            "delivered_at < :now - {}",
            history::retention_secs()
        ))
}

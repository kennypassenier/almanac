//! K20 — where Almanac keeps its state, resolved in one place.
//!
//! Standing rule 28: *state has an address, and Kenny owns it.* One
//! documented knob moves the whole tree, and every path is derived from
//! that root rather than composed independently — "a path assembled in
//! three places is three places that can disagree once the root moves".
//!
//! Almanac had four independent settings whose defaults happened to
//! form a coherent tree. Happening to agree is not the same as being
//! derived: the homelab tried to move almanac onto a bind-mounted host
//! directory on 2026-08-31 and could not, because there was no single
//! thing to move.
//!
//! Resolution is a pure function of the environment so it can be tested
//! without setting process-wide variables, which is also why it takes a
//! lookup closure rather than reading `std::env` itself.

use std::path::{Path, PathBuf};

/// The one knob. Everything below is derived from it.
pub const STATE_DIR_ENV: &str = "ALMANAC_STATE_DIR";

/// Per-path overrides, kept because deployments already set them and
/// breaking a running service to tidy an interface is the wrong trade.
/// A specific setting wins over the root — the more precise instruction
/// is the one the operator meant.
pub const PROFILES_DIR_ENV: &str = "ALMANAC_PROFILES_DIR";
pub const DATA_DIR_ENV: &str = "ALMANAC_DATA_DIR";
pub const JOURNAL_ENV: &str = "ALMANAC_JOURNAL";
pub const TOKEN_STORE_ENV: &str = "ALMANAC_TOKEN_STORE";

/// Every path Almanac writes to or reads its own state from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// What the knob was set to; `.` when it was not set.
    pub root: PathBuf,
    /// Mapping profiles — configuration, edited by a person.
    pub profiles_dir: PathBuf,
    /// Durable state: the journal, the sealed tokens, the update state
    /// and the exclusive lock.
    pub data_dir: PathBuf,
    pub journal: PathBuf,
    pub token_store: PathBuf,
}

impl Paths {
    /// Resolves from a lookup, which production fills with
    /// `std::env::var` and tests fill with a map.
    ///
    /// An empty or whitespace-only value counts as unset. That is a
    /// leftover in an env file rather than an instruction to write into
    /// the filesystem root, and treating `ALMANAC_STATE_DIR=` as "put
    /// everything in `/`" is the kind of literal-mindedness that
    /// deletes an afternoon.
    pub fn resolve(get: impl Fn(&str) -> Option<String>) -> Self {
        let value = |key: &str| {
            get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        // "." rather than an absolute default: it reproduces exactly
        // what almanac did before this existed — profiles/ and data/
        // beside the working directory — so a deployment that sets
        // nothing sees no change at all.
        let root = value(STATE_DIR_ENV).map_or_else(|| PathBuf::from("."), PathBuf::from);

        let profiles_dir =
            value(PROFILES_DIR_ENV).map_or_else(|| join(&root, "profiles"), PathBuf::from);
        let data_dir = value(DATA_DIR_ENV).map_or_else(|| join(&root, "data"), PathBuf::from);

        // Derived from the resolved data directory, not from the root:
        // an operator who moves only the data directory expects the
        // journal to follow it, and the journal living somewhere else
        // than the lock that guards it would be a genuine hazard.
        let journal =
            value(JOURNAL_ENV).map_or_else(|| data_dir.join("journal.jsonl"), PathBuf::from);
        let token_store =
            value(TOKEN_STORE_ENV).map_or_else(|| data_dir.join("tokens.json"), PathBuf::from);

        Self {
            root,
            profiles_dir,
            data_dir,
            journal,
            token_store,
        }
    }

    /// The paths a backup has to carry, in the order a reader wants
    /// them. There is no cache to exclude — almanac keeps nothing
    /// regenerable on disk, which rule 28 asks to be stated rather than
    /// left to inference.
    pub fn backed_up(&self) -> [&Path; 2] {
        [&self.profiles_dir, &self.data_dir]
    }
}

/// Keeps `.` out of the rendered path, so the default reads as
/// `profiles` rather than `./profiles` in errors and logs.
fn join(root: &Path, child: &str) -> PathBuf {
    if root == Path::new(".") {
        PathBuf::from(child)
    } else {
        root.join(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn nothing_set_reproduces_exactly_what_almanac_did_before_this_existed() {
        // The compatibility that matters: a deployment that adopts this
        // release without changing anything must see no change at all.
        let p = Paths::resolve(env(&[]));
        assert_eq!(p.profiles_dir, PathBuf::from("profiles"));
        assert_eq!(p.data_dir, PathBuf::from("data"));
        assert_eq!(p.journal, PathBuf::from("data/journal.jsonl"));
        assert_eq!(p.token_store, PathBuf::from("data/tokens.json"));
    }

    #[test]
    fn one_knob_moves_the_whole_tree() {
        // The entire point, and the thing the homelab could not do.
        let p = Paths::resolve(env(&[(STATE_DIR_ENV, "/appdata/almanac")]));
        assert_eq!(p.profiles_dir, PathBuf::from("/appdata/almanac/profiles"));
        assert_eq!(p.data_dir, PathBuf::from("/appdata/almanac/data"));
        assert_eq!(
            p.journal,
            PathBuf::from("/appdata/almanac/data/journal.jsonl")
        );
        assert_eq!(
            p.token_store,
            PathBuf::from("/appdata/almanac/data/tokens.json")
        );
    }

    #[test]
    fn a_specific_setting_wins_over_the_root() {
        // Deployments already set these four, and a release that
        // silently relocated a live journal because a tidier knob
        // appeared would be the worst kind of upgrade.
        let p = Paths::resolve(env(&[
            (STATE_DIR_ENV, "/appdata/almanac"),
            (PROFILES_DIR_ENV, "/etc/almanac/profiles"),
        ]));
        assert_eq!(p.profiles_dir, PathBuf::from("/etc/almanac/profiles"));
        assert_eq!(
            p.data_dir,
            PathBuf::from("/appdata/almanac/data"),
            "overriding one path must not disturb the others"
        );
    }

    #[test]
    fn the_journal_follows_the_data_directory_not_the_root() {
        // Someone who moves only the data directory means the journal
        // too — and a journal separated from the lock that guards it is
        // two processes away from a corrupted log.
        let p = Paths::resolve(env(&[
            (STATE_DIR_ENV, "/appdata/almanac"),
            (DATA_DIR_ENV, "/var/lib/almanac"),
        ]));
        assert_eq!(p.journal, PathBuf::from("/var/lib/almanac/journal.jsonl"));
        assert_eq!(p.token_store, PathBuf::from("/var/lib/almanac/tokens.json"));
    }

    #[test]
    fn the_live_deployments_four_absolute_settings_are_honoured_unchanged() {
        // CT 112 as it stands on 2026-09-01. This release must be a
        // no-op there; the homelab migrates deliberately, later.
        let p = Paths::resolve(env(&[
            (PROFILES_DIR_ENV, "/opt/almanac/profiles"),
            (DATA_DIR_ENV, "/opt/almanac/data"),
            (JOURNAL_ENV, "/opt/almanac/data/journal.jsonl"),
            (TOKEN_STORE_ENV, "/opt/almanac/data/tokens.json"),
        ]));
        assert_eq!(p.profiles_dir, PathBuf::from("/opt/almanac/profiles"));
        assert_eq!(p.data_dir, PathBuf::from("/opt/almanac/data"));
        assert_eq!(p.journal, PathBuf::from("/opt/almanac/data/journal.jsonl"));
        assert_eq!(
            p.token_store,
            PathBuf::from("/opt/almanac/data/tokens.json")
        );
    }

    #[test]
    fn an_empty_value_is_a_leftover_not_an_instruction_to_use_the_filesystem_root() {
        let p = Paths::resolve(env(&[(STATE_DIR_ENV, "   ")]));
        assert_eq!(p.data_dir, PathBuf::from("data"));
        assert_ne!(p.data_dir, PathBuf::from("/data"));
    }

    #[test]
    fn surrounding_whitespace_is_tolerated_the_way_it_is_everywhere_else() {
        let p = Paths::resolve(env(&[(STATE_DIR_ENV, "  /appdata/almanac  ")]));
        assert_eq!(p.data_dir, PathBuf::from("/appdata/almanac/data"));
    }

    #[test]
    fn a_backup_carries_configuration_and_data_and_nothing_else() {
        let p = Paths::resolve(env(&[(STATE_DIR_ENV, "/appdata/almanac")]));
        assert_eq!(
            p.backed_up(),
            [
                Path::new("/appdata/almanac/profiles"),
                Path::new("/appdata/almanac/data")
            ]
        );
    }
}

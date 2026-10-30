//! The release host stand-in, for the self-update tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;

/// The public half of a throwaway minisign key. Its secret half was
/// generated once, used to sign the fixtures in `tests/self_update.rs`,
/// and never kept. It is not the release key and grants nothing.
pub const THROWAWAY_RELEASE_PUBKEY: &str =
    "RWSD7EDZN4XNRaGibu+cfLqrMzCOC0pAyW/CCeNTg5A1BcZMfHalxTx4";

/// A stand-in for the release host.
pub struct ReleaseStub {
    pub base_url: String,
    /// How many times a running instance asked what the latest version
    /// is — the signal that a check actually happened.
    pub version_requests: Arc<AtomicUsize>,
}

impl ReleaseStub {
    pub fn checks(&self) -> usize {
        self.version_requests.load(Ordering::SeqCst)
    }

    /// Serves a release whose manifest does not match its signature —
    /// the shape of a compromised release host, and what the
    /// verification-failure threshold counts.
    pub async fn start_unverifiable() -> Self {
        async fn version(State(count): State<Arc<AtomicUsize>>) -> &'static str {
            count.fetch_add(1, Ordering::SeqCst);
            "0.2.0"
        }
        async fn manifest() -> &'static str {
            // One byte different from what was signed.
            "306d6ca7407560340797866e077e053627ad409277d1b9da58106fce4cf717cb  almanac\n"
        }
        async fn signature() -> &'static str {
            "untrusted comment: signature from minisign secret key\nRUSD7EDZN4XNRWzTd/nVYZPGWVHFhAdBbUgEuUgnG8PNvVN24ZFFhkXVQRLam6HmQw9bcUwAMGbUi7Rgew5LWC0DGlLbmbcy9g8=\ntrusted comment: timestamp:1787940761\tfile:SHA256SUMS\thashed\nsR1goffxRR5oeoS6no0+GjFuKrSt4UannaugGwWSe5Ahv3f5bAmd7nCCRA/a/sYW5TO00kNiMYb2yXKigUm5CA==\n"
        }

        let count = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/download/v0.2.0/SHA256SUMS", axum::routing::get(manifest))
            .route(
                "/download/v0.2.0/SHA256SUMS.minisig",
                axum::routing::get(signature),
            )
            .route(
                "/latest/download/VERSION",
                axum::routing::get(version).with_state(Arc::clone(&count)),
            );

        let address = super::serve_loopback(router).await;

        Self {
            base_url: format!("http://{address}"),
            version_requests: count,
        }
    }
}

//! The OAuth2 token endpoint stand-in and the throwaway key that signs
//! for it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use serde_json::{Value, json};

/// A stand-in for Google's OAuth2 token endpoint.
pub struct TokenStub {
    pub url: String,
    pub state: Arc<TokenStubState>,
}

#[derive(Default)]
pub struct TokenStubState {
    /// How many token requests arrived. The number that proves AR18:
    /// twenty concurrent callers must produce one.
    pub hits: AtomicUsize,
    /// Seconds each issued token is valid for.
    pub expires_in: AtomicUsize,
}

impl TokenStubState {
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

async fn issue_token(State(state): State<Arc<TokenStubState>>) -> axum::Json<Value> {
    let n = state.hits.fetch_add(1, Ordering::SeqCst);
    let expires_in = state.expires_in.load(Ordering::SeqCst);
    // A different token each time, so a test can tell a reused token
    // from a refreshed one.
    axum::Json(json!({"access_token": format!("stub-token-{n}"), "expires_in": expires_in}))
}

impl TokenStub {
    /// Starts the stub. `expires_in` is what it reports for each
    /// issued token — pass something small to make a token expire
    /// without waiting.
    pub async fn start(expires_in: usize) -> Self {
        let state = Arc::new(TokenStubState::default());
        state.expires_in.store(expires_in, Ordering::SeqCst);

        let router = Router::new()
            .route("/token", axum::routing::post(issue_token))
            .with_state(Arc::clone(&state));

        let address = super::serve_loopback(router).await;

        Self {
            url: format!("http://{address}/token"),
            state,
        }
    }
}

/// A throwaway RSA key generated for this test suite alone. It signs
/// JWTs that only ever reach the local stub above, and grants access to
/// nothing. Kept here rather than duplicated per test file.
pub const TEST_ONLY_THROWAWAY_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCzEluLXIZ/BS/L
w29MNxikrQDpi6rp68cJ+hbiaSHdBRWx3fJqnzlxhl4PtVKyef7zVa9WcvhgOzSi
ZzCwEUprF8vAVMbFQOhPK1xSh3YuxpH9RoFceuoPk1B6j1SuJqkX65e5KaeI4kby
CMwDbwX63WNl2aEKOnc+/U4Q4E/BXTxeVrhLuVdsm7KNrxDzKcKuddPpQUjS94nn
JiHlWXp8CG2ALeuvAxUg+2NCqPU4jq0oGU6De701k15ZT2qRyPTwvmGKYqULvVSl
Dvg/34GiwtR+8AtiZATK5NMdXbTEC+rznHzNJV82cjAckPl+yZpKFhosTMSP0Prv
FN+azeDnAgMBAAECggEAMLqsIq5ZAzO8H+zc2pabpCRX/TW+ms1IapSdqZsGVgjO
MIq/LviJPzVbX1buXBcKo9kLT7EVmcpCtnbyLtdlsuLU1U+8j2zsSq73/pVSOcRb
cdq/1RS1oOtrmQ5r8sAef53iucZ2Cq/YsoBmVADgVbXtGIgyZIAodwGjPsBrs6hg
WjQ0D8sd7seogE3s4kPNsBmf+8dgNox4cMONg5up4Xehxl/98dBFFHDTUgKZOSsj
pVHp/OeNKi6PtmZSmW7MxSFpisAnYH94nnDyuWJ7a/N470eybvyRRF1B0l3v09sB
JmigaIuffpACCEeoj1/4zvje02uyXVIoMfSbMetwMQKBgQDt4/qpf4ZjBb8pUb3v
SCu6Zix0dqIZ09NYd8nShv5j70PnMDnxnrq6dyXJAVGvfWrQ2O1HxqlZpsKH9yeH
feQEx5ZzAua+fzcoNaEop2LBlOEx6vw0zbsQGG3I2+3DezmcZ38ojs8wLyKZToD9
NFo0dTKD+eUp0yoRtudMJab0cwKBgQDAtB1TChueN97Qfa4MvHdjCJc7I5+OkhgQ
SW+R5lz2Jk+/GVn53YzmH18JdnRAQDsTG8nv2DePJGxiq+jFFIF8jL7gB1vyBfOu
5CNZ6W4Rd6IeGlTLFlM5/N8qalKZUoq/wZMGbLrz09ephOpzVUA51eYATUw4DHwc
zusntFT4vQKBgQC2eh0JrX+JL5xN9pzKEkMwrTVGdMWtKBZDE0flzJUQVTVx/kVE
OOylIcYDJJbjFUI9R1jjqNi4ozkvEH/q579jhzG5sS0MTQsjNdgUFimjsi73mnex
jWoDU6nK3CDKxRgRCDa7BqiZHl7c2CILl//lo0yHfcWySn9HrVRIzcz+TwKBgEns
jpdNeFzQyAwpOnyuTApUwFcyikISL2MIGOHagmz3M352xjqBUEzzWezyYRRIz6C7
91KoGmAyM9YCZrA79pSGFa8xg4cr21iLMjiKwOu4fhuYNFEYRmMna6EE2pzwukNn
ifRb/7gL216vm5UU7ieBs9MH1CZoO7B9fF5l4nbtAoGAL53WIGpZ9qMrn0v+6YA/
eZxhaEs2l12zgGgk1sJ4wIPNqqdtInRMlih+w4HrSsiVVuLlyEhPo730MZ1gILSC
ibQwm/ptqIi9PyZwuYkV4lrxsBZfuHo2UUvhheQ14CRCahoyS3BmE9sGgltWuJK5
Mv4b6oszn6H7ZA5VPrqSeE8=
-----END PRIVATE KEY-----";

/// Credentials pointing at a token stub, signed with that key.
pub fn stub_credentials(token_url: &str) -> crate::core::auth::ServiceAccountCredentials {
    crate::core::auth::ServiceAccountCredentials {
        client_email: "stub@example.iam.gserviceaccount.com".to_string(),
        private_key: TEST_ONLY_THROWAWAY_KEY.to_string(),
        token_url: token_url.to_string(),
    }
}

//! `KitHub`: the running kit hub and the requests a test sends it.

use std::net::SocketAddr;
use std::sync::Arc;

use almanac::shell::state::AppState;
use almanac::shell::testing::{CalendarStub, TokenStub};
use chassis::testing::TestApp;
use reqwest::Method;

pub struct KitHub {
    pub addr: SocketAddr,
    pub state: Arc<AppState>,
    pub calendar: CalendarStub,
    pub dir: tempfile::TempDir,
    pub(super) app: TestApp,
    pub(super) _tokens: TokenStub,
}

impl KitHub {
    pub fn url(&self, path: &str) -> String {
        self.app.url(path)
    }

    /// This hub's `ALMANAC_TOKEN` — the fixed `TOKEN` pinned through
    /// `extra_env`, which the kit's harness honours since chassis-rs
    /// 2.0.0 (CF-12). Never the fixed `KEY`.
    pub fn token(&self) -> &str {
        self.app.token()
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("a client")
    }

    /// A browser with the admin session.
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.app
            .request(Method::GET, path)
            .send()
            .await
            .expect("a response")
    }

    pub async fn get_anon(&self, path: &str) -> reqwest::Response {
        Self::http()
            .get(self.url(path))
            .send()
            .await
            .expect("a response")
    }

    pub async fn page(&self, path: &str) -> String {
        let response = self.get(path).await;
        assert_eq!(response.status(), 200, "{path} must render");
        response.text().await.expect("a body")
    }

    /// One of the dashboard's own forms, posted by the admin's browser.
    pub async fn form(&self, path: &str, body: &str) -> reqwest::Response {
        self.app
            .request(Method::POST, path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_string())
            .send()
            .await
            .expect("a response")
    }

    /// A script with a bearer token.
    pub fn bearer(&self, method: Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.app.bearer(method, path, token)
    }

    pub async fn post_json(
        &self,
        path: &str,
        token: Option<&str>,
        body: &str,
    ) -> reqwest::Response {
        let mut request = Self::http()
            .post(self.url(path))
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.send().await.expect("a response")
    }

    /// Issue a client token on the kit's Sources page (as the admin) and
    /// return the token, the way a person would with Reveal.
    pub async fn issue_client(&self, name: &str) -> String {
        // 4.0.2: the issue form carries the calendar; a source with a
        // profile on disk keeps it, a new one gets this test calendar.
        self.app
            .issue_client(name, &[("calendar", "cal-test")])
            .await
            .token
    }

    pub async fn shutdown(mut self) {
        self.app.shutdown().await;
    }
}

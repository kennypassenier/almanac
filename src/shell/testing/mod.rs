//! Test-only stubs for the two systems Almanac talks to (T0).
//!
//! Before this existed there was no way to stand between Almanac and
//! Google, so the retry loop, the per-profile calendar routing and a
//! successful token refresh were all unreachable for any test that did
//! not use Kenny's real calendar. That is why three Essential features
//! leaned entirely on two `#[ignore]`d live tests — and why a
//! connection failure sat misclassified inside the retry loop until an
//! audit read it line by line.
//!
//! Both stubs are real HTTP servers on a loopback port, so the code
//! under test performs genuine requests: no mocked client, no
//! behaviour that exists only in tests.
//!
//! Compiled into the library rather than gated behind `cfg(test)`:
//! integration tests in `tests/` link the library as an ordinary
//! dependency and cannot see a test-gated module, and K21's dashboard
//! tests have to create a calendar without reaching Google. The
//! alternative was a second hand-rolled stub under `tests/` — the same
//! fixture maintained twice, which is the shape of every drift this
//! project has had to repair.
//!
//! One file per stub: `calendar` (with its route handlers in
//! `calendar/handlers.rs`), `token`, `notify` and `release`.

use std::net::SocketAddr;

use axum::Router;

mod calendar;
mod notify;
mod release;
mod token;

pub use calendar::{CalendarState, CalendarStub, Created};
pub use notify::NotifyStub;
pub use release::{ReleaseStub, THROWAWAY_RELEASE_PUBKEY};
pub use token::{TEST_ONLY_THROWAWAY_KEY, TokenStub, TokenStubState, stub_credentials};

/// Serves `router` on a fresh loopback port and returns its address.
/// Every stub starts this way, so a failure to bind reads the same in
/// all of them.
async fn serve_loopback(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a test stub could not bind a loopback port");
    let address = listener
        .local_addr()
        .expect("a bound loopback listener has an address");
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });
    address
}

//! The I/O-performing shell — HTTP handlers, the Google Calendar
//! client, config/profile file loading (AR13). The only place allowed
//! to import reqwest or perform file/network I/O directly; whatever it
//! learns from the outside world reaches `core` through explicit
//! function calls and trait implementations, never the reverse.
//!
//! Empty at L0 (walking skeleton) — routes and the real client land
//! starting with milestone L1 (calendar core) through L4 (sources &
//! visibility).

use axum::Router;

/// Builds the application's Axum router.
pub fn build_router() -> Router {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_router_does_not_panic() {
        let _ = build_router();
    }
}

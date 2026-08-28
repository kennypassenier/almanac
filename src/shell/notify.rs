//! Operational notifications (AR23, AR24, AR26).
//!
//! Almanac does not build a notification channel of its own. Home
//! Assistant already has one for homelab operations —
//! `automation.homelab_ops_webhook` — which archives every event to
//! `/media/homelab_events.log`, mirrors it to the logbook for the
//! Homelab dashboard tab, and pushes *only failures*, and only when
//! `input_boolean.homelab_event_notifications` is on. Almanac posting
//! there means its events land in the same stream as every other
//! homelab operation, inherit Do Not Disturb, the active-hours
//! schedule and the acknowledgement bus, and honour the one toggle
//! that decides how loud homelab failures are.
//!
//! Two consequences worth stating, because both are deliberate:
//!
//! - **A successful self-update is a log line, not an interruption.**
//!   It is still sent — the archive is the point — but with `ok: true`
//!   it never reaches the phone.
//! - **The webhook id is the only authentication**, and the webhook is
//!   `local_only`. That is an accepted trade for a log feed, and the
//!   reason nothing on the Home Assistant side acts on Almanac's
//!   payload. Never send anything here that must be authentic, and
//!   never send a secret: this leaves the process unencrypted over the
//!   LAN and lands in a file and a logbook that are not treated as
//!   sensitive.

use std::time::Duration;

use serde::Serialize;

/// Where to post. Absent means notifications are disabled — Almanac
/// runs perfectly well without them, so this is a warning at startup
/// and never a failure.
pub const WEBHOOK_ENV: &str = "ALMANAC_NOTIFY_WEBHOOK";

/// A notification never blocks operational work, so it gets a short
/// timeout of its own rather than inheriting the client's.
const TIMEOUT: Duration = Duration::from_secs(5);

/// One homelab operation event, in the shape
/// `automation.homelab_ops_webhook` already accepts.
#[derive(Debug, Serialize)]
pub struct Event {
    /// Stable per-situation identifier. Home Assistant derives
    /// `ack_id: homelab_<op>` from this and uses it as the push tag,
    /// so a repeated failure of the same kind collapses into one line
    /// on the lock screen instead of stacking. Keep these stable and
    /// specific for that reason.
    pub op: &'static str,
    pub ok: bool,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Operation names Almanac uses. Constants rather than string
/// literals at the call sites: the value doubles as the notification's
/// deduplication key, so a typo would quietly split one alert into two.
pub mod ops {
    /// A new version was verified, installed and the process is
    /// restarting into it.
    pub const UPDATE_APPLIED: &str = "almanac-update";
    /// The new version did not come up healthy and the previous binary
    /// was put back (AR23). Never silent.
    pub const UPDATE_REVERTED: &str = "almanac-update-reverted";
    /// A release failed signature or checksum verification (AR24).
    /// Either the release host is compromised or the signing key
    /// changed; both need a human.
    pub const UPDATE_UNVERIFIED: &str = "almanac-update-unverified";
    /// The journal is filling up because deliveries keep failing
    /// (AR26) — warn while there is still room, not once events are
    /// already being refused.
    pub const JOURNAL_BACKLOG: &str = "almanac-journal-backlog";
}

/// Posts operational events to Home Assistant.
///
/// Cloneable and cheap: the underlying `reqwest::Client` is a handle
/// to a shared connection pool.
#[derive(Clone)]
pub struct Notifier {
    http: reqwest::Client,
    webhook: Option<String>,
}

impl Notifier {
    /// Reads the webhook URL from the environment. An unset or blank
    /// value disables notifications with a warning: running blind is a
    /// worse default than not running, so it is said out loud once.
    pub fn from_env(http: reqwest::Client) -> Self {
        match std::env::var(WEBHOOK_ENV) {
            Ok(url) if !url.trim().is_empty() => Self {
                http,
                webhook: Some(url.trim().to_string()),
            },
            _ => {
                tracing::warn!(
                    "{WEBHOOK_ENV} is not set — a reverted self-update, a release that fails \
                     verification and a filling journal will only appear in the log. Set it to \
                     the Home Assistant homelab-ops webhook URL to be told about them."
                );
                Self {
                    http,
                    webhook: None,
                }
            }
        }
    }

    /// An explicit destination — used by tests and by anything that
    /// already knows the URL.
    pub fn to(http: reqwest::Client, webhook: impl Into<String>) -> Self {
        Self {
            http,
            webhook: Some(webhook.into()),
        }
    }

    /// A notifier that sends nothing, for contexts with no channel.
    pub fn disabled() -> Self {
        Self {
            http: reqwest::Client::new(),
            webhook: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.webhook.is_some()
    }

    /// Sends one event. Never fails the caller and never panics: the
    /// situations this reports are already bad, and a notification
    /// that cannot be delivered must not take down the thing it was
    /// reporting about.
    pub async fn send(&self, event: Event) {
        let Some(url) = &self.webhook else {
            tracing::info!(
                op = event.op, ok = event.ok, version = %event.version,
                error = ?event.error,
                "operational event (no notification webhook configured)"
            );
            return;
        };

        let result = self
            .http
            .post(url)
            .timeout(TIMEOUT)
            .json(&event)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {
                tracing::info!(op = event.op, ok = event.ok, "notified home assistant");
            }
            Ok(response) => {
                // A webhook answers 200 whether or not the automation
                // did anything, so anything else means it never
                // arrived at all.
                tracing::warn!(
                    op = event.op,
                    status = %response.status(),
                    "the notification webhook refused the event — check that the automation is on \
                     and that this host is on the LAN, since the webhook is local_only"
                );
            }
            Err(e) => {
                tracing::warn!(
                    op = event.op, error = %e,
                    "could not reach the notification webhook; the event is in this log only"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notifier_with_no_webhook_is_disabled_rather_than_broken() {
        let notifier = Notifier::disabled();
        assert!(!notifier.is_enabled());
    }

    #[tokio::test]
    async fn sending_without_a_webhook_is_a_no_op_not_a_panic() {
        // The paths that notify are already failure paths; a missing
        // webhook must not turn a reverted update into a crash.
        Notifier::disabled()
            .send(Event {
                op: ops::UPDATE_REVERTED,
                ok: false,
                version: "0.2.0".to_string(),
                error: Some("did not come up".to_string()),
            })
            .await;
    }

    #[test]
    fn the_payload_matches_the_shape_home_assistant_already_parses() {
        // The automation reads op/ok/version/error and defaults the
        // last two. A renamed field would silently produce log lines
        // with no version and no reason.
        let json = serde_json::to_value(Event {
            op: ops::UPDATE_APPLIED,
            ok: true,
            version: "0.2.0".to_string(),
            error: None,
        })
        .unwrap();

        assert_eq!(json["op"], "almanac-update");
        assert_eq!(json["ok"], true);
        assert_eq!(json["version"], "0.2.0");
        assert!(
            json.get("error").is_none(),
            "a successful event must not carry an empty error field — the log line would read \
             'error: null'"
        );
    }

    #[test]
    fn a_failure_carries_its_reason() {
        let json = serde_json::to_value(Event {
            op: ops::UPDATE_UNVERIFIED,
            ok: false,
            version: "0.2.0".to_string(),
            error: Some("signature does not verify".to_string()),
        })
        .unwrap();

        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "signature does not verify");
    }

    #[test]
    fn the_operation_names_are_distinct_so_alerts_do_not_collapse_into_each_other() {
        // Home Assistant dedupes pushes by op. Two different problems
        // sharing a name would mean the second never reaches the phone.
        let all = [
            ops::UPDATE_APPLIED,
            ops::UPDATE_REVERTED,
            ops::UPDATE_UNVERIFIED,
            ops::JOURNAL_BACKLOG,
        ];
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }
}

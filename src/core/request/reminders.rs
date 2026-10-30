//! The reminders a source asks for, checked against Google's limits.

use crate::core::calendar::{ReminderOverride, Reminders};
use crate::core::error::AlmanacError;

use super::ReminderRequest;

/// Google's own limits on reminder overrides. Checked here so a source
/// gets a message naming the field instead of a Google 400.
const MAX_REMINDERS: usize = 5;
const MAX_REMINDER_MINUTES: i64 = 4 * 7 * 24 * 60;

pub(super) fn to_reminders(
    asked: &ReminderRequest,
    origin: &str,
) -> Result<Reminders, AlmanacError> {
    let count = asked.popup_minutes_before.len() + asked.email_minutes_before.len();
    if asked.silent && count > 0 {
        return Err(AlmanacError::ProfileValidation {
            message: format!("{origin}: \"reminders\" is silent and also lists reminders"),
            remedy: "either set silent = true, or give the minutes — not both".to_string(),
        });
    }
    if count > MAX_REMINDERS {
        return Err(AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: \"reminders\" asks for {count}; Google allows {MAX_REMINDERS}"
            ),
            remedy: format!("keep at most {MAX_REMINDERS} reminders across popup and email"),
        });
    }
    for minutes in asked
        .popup_minutes_before
        .iter()
        .chain(&asked.email_minutes_before)
    {
        if *minutes < 0 || *minutes > MAX_REMINDER_MINUTES {
            return Err(AlmanacError::ProfileValidation {
                message: format!("{origin}: \"reminders\" asks for {minutes} minutes before"),
                remedy: format!(
                    "Google allows 0 to {MAX_REMINDER_MINUTES} minutes (four weeks) before the start"
                ),
            });
        }
    }
    // Silence and overrides both mean "not the calendar's default";
    // the difference is whether the list is empty. Omitting the block
    // entirely is the third outcome and never reaches here.
    Ok(Reminders {
        use_default: false,
        overrides: asked
            .popup_minutes_before
            .iter()
            .map(|m| ReminderOverride {
                method: "popup".to_string(),
                minutes: *m as u32,
            })
            .chain(asked.email_minutes_before.iter().map(|m| ReminderOverride {
                method: "email".to_string(),
                minutes: *m as u32,
            }))
            .collect(),
    })
}

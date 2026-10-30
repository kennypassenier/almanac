//! The optional short mark a profile gives its source.

use crate::core::error::AlmanacError;

/// The longest mark a source may carry; the page draws it in a small badge.
const MAX_MARK_CHARS: usize = 3;

/// A mark, when given, is one to three letters or digits.
pub(super) fn validate_mark(mark: Option<&str>, origin: &str) -> Result<(), AlmanacError> {
    let Some(mark) = mark else { return Ok(()) };
    let count = mark.chars().count();
    if count == 0 || count > MAX_MARK_CHARS || !mark.chars().all(|c| c.is_alphanumeric()) {
        return Err(AlmanacError::ProfileValidation {
            message: format!(
                "{origin}: mark {mark:?} is not 1 to {MAX_MARK_CHARS} letters or digits"
            ),
            remedy: "use a short mark such as \"HA\", or leave mark out".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_mark_or_none_is_accepted() {
        assert!(validate_mark(None, "test").is_ok());
        assert!(validate_mark(Some("HA"), "test").is_ok());
    }

    #[test]
    fn an_empty_long_or_punctuated_mark_is_refused() {
        for bad in ["", "HOME", "H-A"] {
            assert!(
                validate_mark(Some(bad), "test").is_err(),
                "{bad:?} must be refused"
            );
        }
    }
}

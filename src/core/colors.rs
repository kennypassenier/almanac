//! Google's event colour palette, by the names Google itself shows.
//!
//! Google's API takes a `colorId`: the string `"1"` through `"11"`. Its
//! UI shows names — Tomato, Sage, Peacock — and nothing in between
//! translates. The consequence was visible in this repository: the
//! `grafana` profile asked for `"tomato"` and `"sage"`, which Google
//! would have refused or ignored, and nobody noticed because that
//! profile never sent an event in its life.
//!
//! So a source names a colour and this maps it. An id straight through
//! is still accepted — someone who knows `"11"` should not be told they
//! are wrong — and anything else is refused by name rather than
//! silently producing an event in the calendar's default colour, which
//! is indistinguishable from having asked for nothing.

/// The eleven colours, in Google's own order, as `(id, name)`.
const PALETTE: [(&str, &str); 11] = [
    ("1", "lavender"),
    ("2", "sage"),
    ("3", "grape"),
    ("4", "flamingo"),
    ("5", "banana"),
    ("6", "tangerine"),
    ("7", "peacock"),
    ("8", "graphite"),
    ("9", "blueberry"),
    ("10", "basil"),
    ("11", "tomato"),
];

/// Resolves a requested colour to Google's `colorId`.
///
/// Case-insensitive on names; ids pass through unchanged.
pub fn resolve(requested: &str) -> Option<&'static str> {
    let wanted = requested.trim().to_ascii_lowercase();
    PALETTE
        .iter()
        .find(|(id, name)| *id == wanted || *name == wanted)
        .map(|(id, _)| *id)
}

/// Every accepted name, for error messages that tell someone what they
/// could have written instead of only what they got wrong.
pub fn names() -> String {
    PALETTE
        .iter()
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_the_id_google_wants() {
        assert_eq!(resolve("tomato"), Some("11"));
        assert_eq!(resolve("Sage"), Some("2"));
        assert_eq!(resolve("  BASIL  "), Some("10"));
    }

    #[test]
    fn an_id_passes_straight_through() {
        assert_eq!(resolve("11"), Some("11"));
        assert_eq!(resolve("1"), Some("1"));
    }

    #[test]
    fn something_that_is_neither_is_refused_rather_than_defaulted() {
        // An unknown colour that silently became "no colour" would be
        // indistinguishable from having asked for nothing — which is
        // exactly how the grafana profile's "tomato" went unnoticed.
        assert_eq!(resolve("puce"), None);
        assert_eq!(resolve("12"), None);
        assert_eq!(resolve(""), None);
    }

    #[test]
    fn the_error_text_lists_what_is_accepted() {
        let names = names();
        assert!(names.contains("tomato"));
        assert!(names.contains("lavender"));
        assert_eq!(names.split(", ").count(), 11);
    }
}

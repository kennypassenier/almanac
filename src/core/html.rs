//! HTML escaping for the dashboard (M12). Pure, and deliberately its
//! own module with its own tests, because the dashboard renders things
//! Almanac did not author: captured webhook bodies (M11) and their
//! headers arrive from whatever system was pointed at the capture
//! endpoint. Rendering those unescaped would turn the debugging tool
//! into a way to run script in the operator's browser.

/// Escapes the five characters that can change the meaning of markup.
///
/// `&` first, or the escapes introduced by the later replacements would
/// themselves be escaped again.
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_is_unchanged() {
        assert_eq!(escape("Wasmachine klaar"), "Wasmachine klaar");
    }

    #[test]
    fn a_script_tag_in_a_captured_body_renders_inert() {
        // The actual attack this exists to stop: someone points a
        // webhook carrying this at the capture endpoint, and the
        // operator later opens the dashboard.
        let escaped = escape("<script>alert(1)</script>");
        assert!(!escaped.contains("<script"));
        assert_eq!(escaped, "&lt;script&gt;alert(1)&lt;/script&gt;");
    }

    #[test]
    fn quotes_are_escaped_so_an_attribute_cannot_be_broken_out_of() {
        assert_eq!(escape(r#"" onmouseover="x"#), "&quot; onmouseover=&quot;x");
        assert_eq!(escape("' onfocus='x"), "&#x27; onfocus=&#x27;x");
    }

    #[test]
    fn ampersands_are_escaped_once_not_twice() {
        // Escaping & last instead of first would turn "<" into
        // "&amp;lt;" and display the escape sequence to the operator.
        assert_eq!(escape("a & b < c"), "a &amp; b &lt; c");
    }

    #[test]
    fn non_ascii_is_preserved() {
        assert_eq!(escape("héllo — wereld"), "héllo — wereld");
    }

    #[test]
    fn an_empty_string_stays_empty() {
        assert_eq!(escape(""), "");
    }
}

//! Shared dashboard utilities.

/// Escape a JSON string so it can be safely embedded inside a `<script>` tag.
///
/// JSON itself is unaware of HTML; embedding raw JSON in a script element lets
/// substrings like `</script>` or HTML comment markers terminate the script
/// block (XSS) and Unicode line separators (U+2028/U+2029) break some parsers.
/// This function rewrites those characters as their `\uXXXX` JSON escapes,
/// which keeps the value valid JSON (so API consumers still parse it normally)
/// while making it safe to interpolate into HTML script bodies.
///
/// We only touch a small set of characters; the rest of the string is left
/// untouched so the common fast path is essentially a memcpy.
#[must_use]
pub fn script_safe_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_script_terminator() {
        let escaped = script_safe_json(r#"{"x":"</script>"}"#);
        assert!(!escaped.contains("</script>"));
        assert!(escaped.contains("\\u003c/script\\u003e"));
        // Round-trips through serde_json
        let v: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(v["x"], "</script>");
    }

    #[test]
    fn escapes_html_metachars_and_separators() {
        let escaped = script_safe_json("<&>\u{2028}\u{2029}");
        assert_eq!(escaped, "\\u003c\\u0026\\u003e\\u2028\\u2029");
    }

    #[test]
    fn passes_through_safe_input() {
        let s = r#"{"a":1,"b":"hello"}"#;
        assert_eq!(script_safe_json(s), s);
    }
}

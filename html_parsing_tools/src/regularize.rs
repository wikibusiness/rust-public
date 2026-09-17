use deunicode::deunicode_char;
use html_escape::decode_html_entities;
use lazy_static::lazy_static;
use regex::Regex;
use unicode_general_category::{get_general_category, GeneralCategory};
use unicode_normalization::UnicodeNormalization;

lazy_static! {
    // Shape of one HTML entity reference, tested individually below rather
    // than decoding the whole string in one pass -- decode_html_entities'
    // own output for a *resolved* entity can itself look entity-shaped
    // again ("&amp;amp;" -> "&amp;", a real ampersand-literal), so a
    // blanket "anything still matching this after decoding" pass would
    // wrongly eat that too. Testing each match against its own decode
    // result tells resolved from unresolved apart instead.
    static ref ENTITY_RE: Regex = Regex::new(r"&#?\w+;").unwrap();
}

fn is_control_like(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::PrivateUse
            | GeneralCategory::Surrogate
            | GeneralCategory::Unassigned
    )
}

// Python's unidecode() no-ops on anything already ASCII (<0x80), including
// C0 control characters -- only non-ASCII "C"-category characters (format
// marks like soft hyphen/BOM, private-use, surrogate, unassigned) actually
// go through its transliteration table, which for pure formatting
// characters usually resolves to "". deunicode's Rust table doesn't carry
// that same ASCII passthrough (it has its own entry for U+0000, etc.), so
// the ASCII check has to happen here, not inside deunicode_char.
fn unidecode_char(c: char) -> String {
    if !is_control_like(c) || c.is_ascii() {
        return c.to_string();
    }
    deunicode_char(c).unwrap_or("").to_string()
}

/// Rust port of oceanai/utils/text_nodes.py's `regularize`: HTML-entity
/// decode, NFKC normalize, transliterate stray control/format characters
/// away, collapse whitespace. Applied to text already extracted from a
/// parsed HTML tree (which the HTML parser itself already entity-decoded
/// once) -- this second pass exists for entities that survive as literal
/// text after that first decode (most commonly double-escaped source HTML,
/// e.g. `&amp;eacute;` -> `&eacute;` -> `é`).
pub fn regularize(text: &str) -> String {
    // w3lib.replace_entities (what this is porting) silently drops any
    // reference it can't resolve -- out-of-Unicode-range numeric refs and
    // unknown named entities both -- to "" in place, rather than leaving
    // it as literal text like html-escape's decode_html_entities does on
    // its own (verified empirically: replace_entities doesn't actually
    // raise OverflowError with the pinned w3lib version, despite
    // text_nodes.py's own _remove_numeric_overflow fallback assuming it
    // does -- that fallback path is dead code today, not exercised here).
    let decoded = ENTITY_RE.replace_all(text, |caps: &regex::Captures| {
        let matched = &caps[0];
        let attempt = decode_html_entities(matched);
        if attempt == matched {
            String::new()
        } else {
            attempt.into_owned()
        }
    });
    let decoded = decoded.into_owned();

    let normalized: String = decoded.nfkc().collect();
    let cleaned: String = normalized.chars().map(unidecode_char).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every case here is a literal captured output from the Python
    // reference implementation (oceanai.utils.text_nodes.regularize) on
    // the same input, not hand-derived -- see the playbook entry for how
    // these were generated.
    #[test]
    fn test_regularize_matches_python_reference() {
        let cases: &[(&str, &str)] = &[
            ("café &eacute;té", "café été"),
            ("&amp;amp; test", "&amp; test"),
            (
                "Tom \\&\\ Jerry &pound;100 &#42030550023695;",
                "Tom \\&\\ Jerry £100",
            ),
            ("full\u{ad}width\u{feff} test", "fullwidth test"),
            (
                "H1            header &pound;100 &#42030550023695;",
                "H1 header £100",
            ),
            ("\u{0}\u{1}\u{2} control chars", "\u{0}\u{1}\u{2} control chars"),
            ("Café naïve résumé", "Café naïve résumé"),
            ("\u{ff21}\u{ff22}\u{ff23}", "ABC"),
            ("naïve", "naïve"),
            ("Bc&#42030550023695;B", "BcB"),
            ("C&#42030550023695;\u{1}", "C\u{1}"),
            ("a \u{feff} Bñ-&#42030550023695;", "a Bñ-"),
            ("&foobar;", ""),
            ("a&unknownentity;b", "ab"),
            ("&#42030550023695;&amp;amp;C3 ", "&amp;C3"),
        ];
        for (input, expected) in cases {
            assert_eq!(&regularize(input), expected, "input: {input:?}");
        }
    }
}

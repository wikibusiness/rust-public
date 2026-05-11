// Pattern normalization — mirrors the Python _cut_version / _preprocess_* functions
// in oceanai/mappings/common/technologies.py

const EMPTY_EXPR: &[&str] = &[
    ".*",
    "(.*)",
    "^(.*)$",
    "^(.+)$",
    "([\\d\\.]+)?",
    "(^.+$)",
];

const JS_MIN_LENGTH: usize = 5;
const STOP_JS_NAME: &str = "analytics";

/// Strip the `\;...` version/confidence suffix.
/// Returns None if the pattern should be dropped entirely (confidence:0 or empty
/// expression paired with a version capture).
pub fn cut_version(text: &str) -> Option<String> {
    let mut parts = text.splitn(2, "\\;");
    let main = parts.next().unwrap_or(text);

    if let Some(suffix) = parts.next() {
        if suffix.contains("confidence:0") {
            return None;
        }
        if EMPTY_EXPR.contains(&main) && suffix.contains("version:") {
            return None;
        }
        return Some(main.to_string());
    }

    Some(text.to_string())
}

/// Replicate Python's `_JS_RE_REPLACEMENTS` substitutions.
pub fn replace_js_regexp(s: &str) -> String {
    s.replace("[^]", "(?:.|\n)").replace('/', "\\/")
}

/// Whether a pattern uses syntax the `regex` crate cannot handle
/// (lookaheads, lookbehinds, backreferences).
pub fn is_unsupported(pattern: &str) -> bool {
    pattern.contains("(?!")
        || pattern.contains("(?=")
        || pattern.contains("(?<!")
        || pattern.contains("(?<=")
        // backreferences \1..\9
        || (1..=9).any(|n| pattern.contains(&format!("\\{n}")))
}

/// Normalise a pattern from a list field (`html`, `script`, `dom`).
/// Returns None if the pattern should be dropped.
pub fn preprocess_list_pattern(text: &str) -> Option<String> {
    let text = cut_version(text)?;
    if is_unsupported(&text) {
        return None;
    }
    let text = if text.starts_with('^') {
        text[1..].to_string()
    } else {
        text
    };
    Some(replace_js_regexp(&text))
}

/// Normalise a pattern from the `js` dict field.
/// Escapes the variable name, appends the pattern, applies JS replacements.
/// Returns None if it should be dropped (too short, stop name, unsupported).
pub fn preprocess_merge_dict_pattern(var_name: &str, value: &str) -> Option<String> {
    if var_name.is_empty() {
        return None;
    }
    let escaped = regex::escape(var_name);
    let suffix = cut_version(value)?;
    let suffix = if suffix.starts_with('^') {
        suffix[1..].to_string()
    } else {
        suffix
    };
    let combined = format!("{escaped}{suffix}");
    let combined = replace_js_regexp(&combined);

    if combined.len() < JS_MIN_LENGTH || combined == STOP_JS_NAME {
        return None;
    }
    if is_unsupported(&combined) {
        return None;
    }
    Some(combined)
}

/// Normalise a value pattern from a dict field (`headers`, `cookies`, `meta`).
/// Returns None if the key should match by presence only (empty/wildcard pattern).
pub fn preprocess_dict_value(value: &str) -> Option<String> {
    if EMPTY_EXPR.contains(&value) || value.is_empty() {
        return None;
    }
    let text = cut_version(value)?;
    if text.is_empty() || is_unsupported(&text) {
        return None;
    }
    Some(replace_js_regexp(&text))
}

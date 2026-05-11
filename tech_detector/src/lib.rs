mod dom;
mod patterns;

use std::collections::HashMap;
use std::collections::HashSet;

use pyo3::prelude::*;
use pyo3::types::PyDict;
use regex::{Regex, RegexSet, RegexSetBuilder};
use serde_json::Value;

/// Compiled matcher for a text-based key (html, script, dom, js).
///
/// Patterns are split into chunks of `chunk_size` so each `RegexSet` stays
/// small enough for the NFA to remain fast. Without chunking a single set of
/// 2 000+ patterns builds an NFA that is an order of magnitude slower than
/// RE2's 59-pattern DFA chunks.
struct TextMatcher {
    sets: Vec<RegexSet>,
    /// sets[i][j] → tech name
    chunk_names: Vec<Vec<String>>,
    total_patterns: usize,
}

impl TextMatcher {
    fn match_parts(&self, parts: &[Vec<u8>]) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        for part in parts {
            let Ok(s) = std::str::from_utf8(part) else {
                continue;
            };
            for (chunk_idx, set) in self.sets.iter().enumerate() {
                for pat_idx in set.matches(s).into_iter() {
                    seen.insert(self.chunk_names[chunk_idx][pat_idx].clone());
                }
            }
        }
        seen.into_iter().collect()
    }
}

/// One entry in a dict-key matcher: optionally a value regex, always a tech name.
struct DictEntry {
    value_regex: Option<Regex>,
    name: String,
}

/// Compiled matcher for a dict-based key (headers, cookies, meta).
struct DictMatcher {
    /// lowercase header/cookie/meta key → entries to check
    entries: HashMap<String, Vec<DictEntry>>,
}

impl DictMatcher {
    fn match_dict(&self, data: &HashMap<String, String>) -> Vec<String> {
        let mut names = Vec::new();
        for (raw_key, value) in data {
            let key = raw_key.to_lowercase();
            if let Some(entries) = self.entries.get(&key) {
                for entry in entries {
                    match &entry.value_regex {
                        None => names.push(entry.name.clone()),
                        Some(re) => {
                            if re.is_match(value) {
                                names.push(entry.name.clone());
                            }
                        }
                    }
                }
            }
        }
        names
    }
}

/// Pre-compiled technology detector.
///
/// Build once at process startup via `TechDetector(json_bytes)`.
/// Thread-safe: all compiled matchers are immutable after construction.
#[pyclass]
pub struct TechDetector {
    html: TextMatcher,
    script: TextMatcher,
    dom: TextMatcher,
    js: TextMatcher,
    headers: DictMatcher,
    cookies: DictMatcher,
    meta: DictMatcher,
    skipped: Vec<String>,
}

// ── Builder helpers ──────────────────────────────────────────────────────────

fn build_text_matcher(
    patterns_and_names: Vec<(String, String)>,
    case_insensitive: bool,
    chunk_size: usize,
    skipped: &mut Vec<String>,
) -> TextMatcher {
    let mut valid: Vec<(String, String)> = Vec::new();

    for (pattern, name) in patterns_and_names {
        let test = RegexSetBuilder::new([&pattern])
            .case_insensitive(case_insensitive)
            .build();
        if test.is_err() {
            skipped.push(format!("{name}: {pattern}"));
            continue;
        }
        valid.push((pattern, name));
    }

    let total_patterns = valid.len();
    let effective_chunk = chunk_size.max(1);
    let mut sets: Vec<RegexSet> = Vec::new();
    let mut chunk_names: Vec<Vec<String>> = Vec::new();

    for chunk in valid.chunks(effective_chunk) {
        let patterns: Vec<&str> = chunk.iter().map(|(p, _)| p.as_str()).collect();
        let names: Vec<String> = chunk.iter().map(|(_, n)| n.clone()).collect();

        let set = RegexSetBuilder::new(&patterns)
            .case_insensitive(case_insensitive)
            .build()
            .unwrap_or_else(|_| RegexSet::empty());

        sets.push(set);
        chunk_names.push(names);
    }

    TextMatcher { sets, chunk_names, total_patterns }
}

fn build_dict_matcher(
    entries_map: HashMap<String, Vec<(Option<String>, String)>>,
    skipped: &mut Vec<String>,
) -> DictMatcher {
    let mut compiled: HashMap<String, Vec<DictEntry>> = HashMap::new();

    for (key, entries) in entries_map {
        let mut dict_entries = Vec::new();
        for (pattern_opt, name) in entries {
            let value_regex = match pattern_opt {
                None => None,
                Some(pat) => match Regex::new(&format!("(?i){pat}")) {
                    Ok(re) => Some(re),
                    Err(_) => {
                        skipped.push(format!("{name}: {pat}"));
                        continue;
                    }
                },
            };
            dict_entries.push(DictEntry { value_regex, name });
        }
        if !dict_entries.is_empty() {
            compiled.insert(key, dict_entries);
        }
    }

    DictMatcher { entries: compiled }
}

// ── JSON parsing ─────────────────────────────────────────────────────────────

fn value_as_strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(arr) => arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

fn parse_apps(
    apps: &serde_json::Map<String, Value>,
) -> (
    Vec<(String, String)>, // html patterns
    Vec<(String, String)>, // script patterns
    Vec<(String, String)>, // dom patterns
    Vec<(String, String)>, // js patterns
    HashMap<String, Vec<(Option<String>, String)>>, // headers
    HashMap<String, Vec<(Option<String>, String)>>, // cookies
    HashMap<String, Vec<(Option<String>, String)>>, // meta
) {
    let mut html_pats: Vec<(String, String)> = Vec::new();
    let mut script_pats: Vec<(String, String)> = Vec::new();
    let mut dom_pats: Vec<(String, String)> = Vec::new();
    let mut js_pats: Vec<(String, String)> = Vec::new();
    let mut headers_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut cookies_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut meta_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();

    for (tech_name, tech_data) in apps {
        let Some(obj) = tech_data.as_object() else {
            continue;
        };

        // ── html ─────────────────────────────────────────────────────────
        if let Some(v) = obj.get("html") {
            for raw in value_as_strings(v) {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    html_pats.push((p, tech_name.clone()));
                }
            }
        }

        // ── script ───────────────────────────────────────────────────────
        if let Some(v) = obj.get("script") {
            for raw in value_as_strings(v) {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    script_pats.push((p, tech_name.clone()));
                }
            }
        }

        // ── dom ──────────────────────────────────────────────────────────
        if let Some(v) = obj.get("dom") {
            let selectors = dom::dom_to_regex(v);
            for raw in selectors {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    dom_pats.push((p, tech_name.clone()));
                }
            }
        }

        // ── js ───────────────────────────────────────────────────────────
        if let Some(Value::Object(js_obj)) = obj.get("js") {
            for (var_name, val) in js_obj {
                let raw_val = val.as_str().unwrap_or("");
                if let Some(p) = patterns::preprocess_merge_dict_pattern(var_name, raw_val) {
                    js_pats.push((p, tech_name.clone()));
                }
            }
        }

        // ── headers / cookies / meta ──────────────────────────────────────
        for (field, target_map) in [
            ("headers", &mut headers_map),
            ("cookies", &mut cookies_map),
            ("meta", &mut meta_map),
        ] {
            if let Some(Value::Object(dict)) = obj.get(field) {
                for (key, val) in dict {
                    let lower_key = key.to_lowercase();
                    let raw_val = match val {
                        Value::String(s) => s.as_str(),
                        Value::Array(arr) => {
                            // take first non-empty string
                            arr.iter().find_map(|v| v.as_str()).unwrap_or("")
                        }
                        _ => "",
                    };
                    let pattern_opt = patterns::preprocess_dict_value(raw_val);
                    target_map
                        .entry(lower_key)
                        .or_default()
                        .push((pattern_opt, tech_name.clone()));
                }
            }
        }
    }

    (
        html_pats,
        script_pats,
        dom_pats,
        js_pats,
        headers_map,
        cookies_map,
        meta_map,
    )
}

// ── PyO3 implementation ──────────────────────────────────────────────────────

#[pymethods]
impl TechDetector {
    /// Build from merged app JSON (bytes).
    ///
    /// `json_data` — serialised `web_applications["apps"]` dict.
    /// `chunk_size` — number of patterns per `RegexSet` chunk (default 256).
    ///   Smaller values reduce per-chunk NFA complexity at the cost of more
    ///   iterations; larger values amortise iteration overhead. Tune with the
    ///   benchmark script.
    #[new]
    #[pyo3(signature = (json_data, chunk_size = 256))]
    pub fn new(json_data: &[u8], chunk_size: usize) -> PyResult<Self> {
        let root: Value = serde_json::from_slice(json_data)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        let apps = match &root {
            Value::Object(obj) if obj.contains_key("apps") => obj["apps"]
                .as_object()
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("'apps' is not an object"))?,
            Value::Object(obj) => obj,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "expected a JSON object",
                ))
            }
        };

        let mut skipped: Vec<String> = Vec::new();

        let (html_pats, script_pats, dom_pats, js_pats, headers_map, cookies_map, meta_map) =
            parse_apps(apps);

        let html = build_text_matcher(html_pats, true, chunk_size, &mut skipped);
        let script = build_text_matcher(script_pats, true, chunk_size, &mut skipped);
        let dom = build_text_matcher(dom_pats, true, chunk_size, &mut skipped);
        let js = build_text_matcher(js_pats, false, chunk_size, &mut skipped); // case-sensitive
        let headers = build_dict_matcher(headers_map, &mut skipped);
        let cookies = build_dict_matcher(cookies_map, &mut skipped);
        let meta = build_dict_matcher(meta_map, &mut skipped);

        Ok(TechDetector {
            html,
            script,
            dom,
            js,
            headers,
            cookies,
            meta,
            skipped,
        })
    }

    /// Match a list of text blobs against a text-based key.
    ///
    /// `key` is one of: "html", "script", "dom", "js"
    /// Returns deduplicated tech names that matched any of the parts.
    pub fn detect_text_key(&self, key: &str, parts: Vec<Vec<u8>>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "html" => &self.html,
            "script" => &self.script,
            "dom" => &self.dom,
            "js" => &self.js,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown text key: {other}"
                )))
            }
        };

        Ok(matcher.match_parts(&parts))
    }

    /// Match a single dict against a dict-based key.
    ///
    /// `key` is one of: "headers", "cookies", "meta"
    /// `data` is a Python dict of str → str.
    pub fn detect_dict_key(&self, key: &str, data: &Bound<'_, PyDict>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "headers" => &self.headers,
            "cookies" => &self.cookies,
            "meta" => &self.meta,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown dict key: {other}"
                )))
            }
        };

        let map: HashMap<String, String> = data
            .iter()
            .filter_map(|(k, v)| {
                let k = k.extract::<String>().ok()?;
                let v = v.extract::<String>().ok()?;
                Some((k, v))
            })
            .collect();

        Ok(matcher.match_dict(&map))
    }

    /// Number of compiled patterns per key (for diagnostics / tests).
    pub fn pattern_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        counts.insert("js".to_string(), self.js.total_patterns);
        counts.insert("script".to_string(), self.script.total_patterns);
        counts.insert("headers".to_string(), self.headers.entries.values().map(|v| v.len()).sum());
        counts.insert("meta".to_string(), self.meta.entries.values().map(|v| v.len()).sum());
        counts.insert("dom".to_string(), self.dom.total_patterns);
        counts.insert("cookies".to_string(), self.cookies.entries.values().map(|v| v.len()).sum());
        counts.insert("html".to_string(), self.html.total_patterns);
        counts
    }

    /// Tech names whose patterns were skipped due to unsupported syntax or
    /// invalid regex. Useful for startup diagnostics.
    pub fn skipped_patterns(&self) -> Vec<String> {
        self.skipped.clone()
    }
}

#[pymodule]
fn tech_detector(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TechDetector>()?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON: &str = r#"{
        "apps": {
            "React": {
                "cats": [12],
                "html": "react",
                "js": {"React.version": "([0-9.]+)\\;version:\\1"}
            },
            "jQuery": {
                "cats": [59],
                "script": "jquery[.\\-]([\\d.]*\\d)[/\\w.]*\\.js\\;version:\\1",
                "js": {"jQuery.fn.jquery": "([\\d.]+)\\;version:\\1"},
                "headers": {"x-powered-by": "jquery"}
            },
            "WordPress": {
                "cats": [1],
                "html": "wp-content",
                "meta": {"generator": "WordPress ([\\d.]+)\\;version:\\1"},
                "cookies": {"wordpress_[a-z0-9_]+": ""}
            }
        }
    }"#;

    fn detector() -> TechDetector {
        TechDetector::new(SAMPLE_JSON.as_bytes(), 256).unwrap()
    }

    #[test]
    fn test_pattern_counts_nonzero() {
        let d = detector();
        let counts = d.pattern_counts();
        assert!(*counts.get("html").unwrap() > 0);
        assert!(*counts.get("script").unwrap() > 0);
        assert!(*counts.get("js").unwrap() > 0);
    }

    #[test]
    fn test_html_detection() {
        let d = detector();
        let parts: Vec<Vec<u8>> = vec![b"<div class='wp-content'>hello</div>".to_vec()];
        let names = d.detect_text_key("html", parts).unwrap();
        assert!(names.contains(&"WordPress".to_string()), "expected WordPress, got {names:?}");
    }

    #[test]
    fn test_html_case_insensitive() {
        let d = detector();
        let parts: Vec<Vec<u8>> = vec![b"React.createElement(App)".to_vec()];
        let names = d.detect_text_key("html", parts).unwrap();
        assert!(names.contains(&"React".to_string()));
    }

    #[test]
    fn test_script_detection() {
        let d = detector();
        let parts: Vec<Vec<u8>> = vec![b"<script src='/jquery-3.6.0.min.js'></script>".to_vec()];
        let names = d.detect_text_key("script", parts).unwrap();
        assert!(names.contains(&"jQuery".to_string()));
    }

    #[test]
    fn test_no_false_positives() {
        let d = detector();
        let parts: Vec<Vec<u8>> = vec![b"<html><body>nothing special here</body></html>".to_vec()];
        let names = d.detect_text_key("html", parts).unwrap();
        assert!(names.is_empty(), "unexpected matches: {names:?}");
    }

    #[test]
    fn test_skipped_patterns_accessible() {
        let d = detector();
        let _ = d.skipped_patterns();
    }

    #[test]
    fn test_cut_version_strips_suffix() {
        assert_eq!(
            patterns::cut_version("somepattern\\;version:\\1"),
            Some("somepattern".to_string())
        );
    }

    #[test]
    fn test_cut_version_drops_confidence_zero() {
        assert_eq!(patterns::cut_version("pattern\\;confidence:0"), None);
    }

    #[test]
    fn test_preprocess_drops_lookahead() {
        assert_eq!(patterns::preprocess_list_pattern("foo(?!bar)"), None);
    }

    #[test]
    fn test_preprocess_strips_caret() {
        let result = patterns::preprocess_list_pattern("^foobar").unwrap();
        assert_eq!(result, "foobar");
    }

    #[test]
    fn test_chunk_size_one() {
        // Each chunk has exactly one pattern — should still work correctly.
        let d = TechDetector::new(SAMPLE_JSON.as_bytes(), 1).unwrap();
        let parts = vec![b"<div class='wp-content'>".to_vec()];
        let names = d.detect_text_key("html", parts).unwrap();
        assert!(names.contains(&"WordPress".to_string()));
    }
}

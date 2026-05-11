mod dom;
mod patterns;

use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::PyDict;
use rayon::prelude::*;
use regex::Regex;
use regex_automata::{
    hybrid::dfa::{DFA, OverlappingState},
    util::syntax::Config as SyntaxConfig,
    Input, MatchKind,
};
use serde_json::Value;

// ── Text matching ─────────────────────────────────────────────────────────────

/// One chunk: a lazy DFA covering `chunk_size` patterns.
///
/// The `DFA` is immutable and `Send + Sync`. Each call to `match_parts` creates
/// a fresh `Cache` per chunk — cheap to allocate, and it warms up as it scans
/// parts, so later parts in the same call benefit from accumulated DFA states.
struct TextChunk {
    dfa: DFA,
    names: Vec<String>, // pattern_index → tech name
}

struct TextMatcher {
    chunks: Vec<TextChunk>,
    total_patterns: usize,
}

impl TextMatcher {
    /// Match all `parts` against all chunks in parallel (one rayon task per chunk).
    ///
    /// Parallelising over chunks (not parts) keeps each thread's `Cache` warm
    /// across all parts, mimicking RE2's per-DFA state caching.
    fn match_parts(&self, parts: &[Vec<u8>]) -> Vec<String> {
        let matched: HashSet<String> = self
            .chunks
            .par_iter()
            .flat_map(|chunk| {
                let mut cache = chunk.dfa.create_cache();
                let n = chunk.names.len();
                let mut names: Vec<String> = Vec::new();

                for part in parts {
                    let Ok(text) = std::str::from_utf8(part) else {
                        continue;
                    };
                    let input = Input::new(text);
                    let mut state = OverlappingState::start();
                    let mut seen = vec![false; n];

                    loop {
                        if chunk
                            .dfa
                            .try_search_overlapping_fwd(&mut cache, &input, &mut state)
                            .is_err()
                        {
                            break;
                        }
                        match state.get_match() {
                            None => break,
                            Some(m) => {
                                let idx = m.pattern().as_usize();
                                if idx < n {
                                    seen[idx] = true;
                                }
                            }
                        }
                    }

                    for (i, &hit) in seen.iter().enumerate() {
                        if hit {
                            names.push(chunk.names[i].clone());
                        }
                    }
                }

                names
            })
            .collect();

        matched.into_iter().collect()
    }
}

// ── Dict matching (headers, cookies, meta) ────────────────────────────────────

struct DictEntry {
    value_regex: Option<Regex>,
    name: String,
}

struct DictMatcher {
    entries: HashMap<String, Vec<DictEntry>>,
}

impl DictMatcher {
    fn match_dict(&self, data: &HashMap<String, String>) -> Vec<String> {
        let mut names = Vec::new();
        for (raw_key, value) in data {
            let key = raw_key.to_lowercase();
            if let Some(entries) = self.entries.get(&key) {
                for entry in entries {
                    let matched = match &entry.value_regex {
                        None => true,
                        Some(re) => re.is_match(value),
                    };
                    if matched {
                        names.push(entry.name.clone());
                    }
                }
            }
        }
        names
    }
}

// ── PyO3 struct ───────────────────────────────────────────────────────────────

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

// ── Builders ──────────────────────────────────────────────────────────────────

fn build_text_matcher(
    patterns_and_names: Vec<(String, String)>,
    case_insensitive: bool,
    chunk_size: usize,
    skipped: &mut Vec<String>,
) -> TextMatcher {
    let syntax = SyntaxConfig::new().case_insensitive(case_insensitive);

    // Validate each pattern individually (fast: only NFA construction).
    let mut valid: Vec<(String, String)> = Vec::new();
    for (pattern, name) in patterns_and_names {
        let ok = DFA::builder()
            .configure(DFA::config().match_kind(MatchKind::All))
            .syntax(syntax)
            .build_many(&[pattern.as_str()])
            .is_ok();
        if ok {
            valid.push((pattern, name));
        } else {
            skipped.push(format!("{name}: {pattern}"));
        }
    }

    let total_patterns = valid.len();
    let effective = chunk_size.max(1);
    let mut chunks: Vec<TextChunk> = Vec::new();

    for chunk in valid.chunks(effective) {
        let pats: Vec<&str> = chunk.iter().map(|(p, _)| p.as_str()).collect();
        let names: Vec<String> = chunk.iter().map(|(_, n)| n.clone()).collect();

        match DFA::builder()
            .configure(DFA::config().match_kind(MatchKind::All))
            .syntax(syntax)
            .build_many(&pats)
        {
            Ok(dfa) => chunks.push(TextChunk { dfa, names }),
            Err(_) => {
                for (p, n) in chunk {
                    skipped.push(format!("{n}: {p}"));
                }
            }
        }
    }

    TextMatcher { chunks, total_patterns }
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
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
    HashMap<String, Vec<(Option<String>, String)>>,
    HashMap<String, Vec<(Option<String>, String)>>,
    HashMap<String, Vec<(Option<String>, String)>>,
) {
    let mut html_pats: Vec<(String, String)> = Vec::new();
    let mut script_pats: Vec<(String, String)> = Vec::new();
    let mut dom_pats: Vec<(String, String)> = Vec::new();
    let mut js_pats: Vec<(String, String)> = Vec::new();
    let mut headers_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut cookies_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut meta_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();

    for (tech_name, tech_data) in apps {
        let Some(obj) = tech_data.as_object() else { continue };

        if let Some(v) = obj.get("html") {
            for raw in value_as_strings(v) {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    html_pats.push((p, tech_name.clone()));
                }
            }
        }

        if let Some(v) = obj.get("script") {
            for raw in value_as_strings(v) {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    script_pats.push((p, tech_name.clone()));
                }
            }
        }

        if let Some(v) = obj.get("dom") {
            for raw in dom::dom_to_regex(v) {
                if let Some(p) = patterns::preprocess_list_pattern(&raw) {
                    dom_pats.push((p, tech_name.clone()));
                }
            }
        }

        if let Some(Value::Object(js_obj)) = obj.get("js") {
            for (var_name, val) in js_obj {
                let raw_val = val.as_str().unwrap_or("");
                if let Some(p) = patterns::preprocess_merge_dict_pattern(var_name, raw_val) {
                    js_pats.push((p, tech_name.clone()));
                }
            }
        }

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
                        Value::Array(arr) => arr.iter().find_map(|v| v.as_str()).unwrap_or(""),
                        _ => "",
                    };
                    let pattern_opt = patterns::preprocess_dict_value(raw_val);
                    target_map.entry(lower_key).or_default().push((pattern_opt, tech_name.clone()));
                }
            }
        }
    }

    (html_pats, script_pats, dom_pats, js_pats, headers_map, cookies_map, meta_map)
}

// ── PyO3 methods ──────────────────────────────────────────────────────────────

#[pymethods]
impl TechDetector {
    /// Build from merged app JSON (bytes).
    ///
    /// `chunk_size` — patterns per lazy-DFA chunk (default 64). Tune with the
    /// benchmark script; optimal depends on core count and pattern complexity.
    #[new]
    #[pyo3(signature = (json_data, chunk_size = 64))]
    pub fn new(json_data: &[u8], chunk_size: usize) -> PyResult<Self> {
        let root: Value = serde_json::from_slice(json_data)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        let apps = match &root {
            Value::Object(obj) if obj.contains_key("apps") => obj["apps"]
                .as_object()
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("'apps' is not an object"))?,
            Value::Object(obj) => obj,
            _ => return Err(pyo3::exceptions::PyValueError::new_err("expected a JSON object")),
        };

        let mut skipped: Vec<String> = Vec::new();
        let (html_pats, script_pats, dom_pats, js_pats, headers_map, cookies_map, meta_map) =
            parse_apps(apps);

        let html = build_text_matcher(html_pats, true, chunk_size, &mut skipped);
        let script = build_text_matcher(script_pats, true, chunk_size, &mut skipped);
        let dom = build_text_matcher(dom_pats, true, chunk_size, &mut skipped);
        let js = build_text_matcher(js_pats, false, chunk_size, &mut skipped);
        let headers = build_dict_matcher(headers_map, &mut skipped);
        let cookies = build_dict_matcher(cookies_map, &mut skipped);
        let meta = build_dict_matcher(meta_map, &mut skipped);

        Ok(TechDetector { html, script, dom, js, headers, cookies, meta, skipped })
    }

    pub fn detect_text_key(&self, key: &str, parts: Vec<Vec<u8>>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "html" => &self.html,
            "script" => &self.script,
            "dom" => &self.dom,
            "js" => &self.js,
            other => return Err(pyo3::exceptions::PyValueError::new_err(format!("unknown key: {other}"))),
        };
        Ok(matcher.match_parts(&parts))
    }

    pub fn detect_dict_key(&self, key: &str, data: &Bound<'_, PyDict>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "headers" => &self.headers,
            "cookies" => &self.cookies,
            "meta" => &self.meta,
            other => return Err(pyo3::exceptions::PyValueError::new_err(format!("unknown key: {other}"))),
        };
        let map: HashMap<String, String> = data
            .iter()
            .filter_map(|(k, v)| Some((k.extract::<String>().ok()?, v.extract::<String>().ok()?)))
            .collect();
        Ok(matcher.match_dict(&map))
    }

    pub fn pattern_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        counts.insert("html".into(), self.html.total_patterns);
        counts.insert("script".into(), self.script.total_patterns);
        counts.insert("dom".into(), self.dom.total_patterns);
        counts.insert("js".into(), self.js.total_patterns);
        counts.insert("headers".into(), self.headers.entries.values().map(|v| v.len()).sum());
        counts.insert("cookies".into(), self.cookies.entries.values().map(|v| v.len()).sum());
        counts.insert("meta".into(), self.meta.entries.values().map(|v| v.len()).sum());
        counts
    }

    pub fn skipped_patterns(&self) -> Vec<String> {
        self.skipped.clone()
    }
}

#[pymodule]
fn tech_detector(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TechDetector>()?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON: &str = r#"{
        "apps": {
            "React": {
                "html": "react",
                "js": {"React.version": "([0-9.]+)\\;version:\\1"}
            },
            "jQuery": {
                "script": "jquery[.\\-]([\\d.]*\\d)[/\\w.]*\\.js\\;version:\\1",
                "js": {"jQuery.fn.jquery": "([\\d.]+)\\;version:\\1"},
                "headers": {"x-powered-by": "jquery"}
            },
            "WordPress": {
                "html": "wp-content",
                "meta": {"generator": "WordPress ([\\d.]+)\\;version:\\1"},
                "cookies": {"wordpress_[a-z0-9_]+": ""}
            }
        }
    }"#;

    fn detector() -> TechDetector {
        TechDetector::new(SAMPLE_JSON.as_bytes(), 64).unwrap()
    }

    #[test]
    fn test_pattern_counts_nonzero() {
        let d = detector();
        let c = d.pattern_counts();
        assert!(*c.get("html").unwrap() > 0);
        assert!(*c.get("script").unwrap() > 0);
        assert!(*c.get("js").unwrap() > 0);
    }

    #[test]
    fn test_html_detection() {
        let d = detector();
        let names = d.detect_text_key("html", vec![b"<div class='wp-content'>".to_vec()]).unwrap();
        assert!(names.contains(&"WordPress".to_string()), "got {names:?}");
    }

    #[test]
    fn test_html_case_insensitive() {
        let d = detector();
        let names = d.detect_text_key("html", vec![b"React.createElement(App)".to_vec()]).unwrap();
        assert!(names.contains(&"React".to_string()));
    }

    #[test]
    fn test_script_detection() {
        let d = detector();
        let names = d
            .detect_text_key("script", vec![b"<script src='/jquery-3.6.0.min.js'>".to_vec()])
            .unwrap();
        assert!(names.contains(&"jQuery".to_string()));
    }

    #[test]
    fn test_no_false_positives() {
        let d = detector();
        let names = d
            .detect_text_key("html", vec![b"<html><body>nothing here</body></html>".to_vec()])
            .unwrap();
        assert!(names.is_empty(), "unexpected: {names:?}");
    }

    #[test]
    fn test_chunk_size_one() {
        let d = TechDetector::new(SAMPLE_JSON.as_bytes(), 1).unwrap();
        let names = d.detect_text_key("html", vec![b"wp-content".to_vec()]).unwrap();
        assert!(names.contains(&"WordPress".to_string()));
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
        assert_eq!(patterns::preprocess_list_pattern("^foobar").unwrap(), "foobar");
    }
}

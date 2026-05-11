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

struct TextChunk {
    dfa: DFA,
    names: Vec<String>,
}

struct TextMatcher {
    chunks: Vec<TextChunk>,
    total_patterns: usize,
}

impl TextMatcher {
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

// ── Dependency resolution (implies) ──────────────────────────────────────────

struct DependencyResolver {
    implies: HashMap<String, Vec<String>>,
}

impl DependencyResolver {
    fn resolve(&self, names: &mut HashSet<String>) {
        let mut queue: Vec<String> = names.iter().cloned().collect();
        while let Some(name) = queue.pop() {
            if let Some(implied) = self.implies.get(&name) {
                for imp in implied {
                    if names.insert(imp.clone()) {
                        queue.push(imp.clone());
                    }
                }
            }
        }
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
    deps: DependencyResolver,
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

/// Strip `\;version:...` suffix from an `implies` entry to get the bare tech name.
fn implies_name(s: &str) -> &str {
    s.splitn(2, "\\;").next().unwrap_or(s)
}

struct ParsedApps {
    html_pats: Vec<(String, String)>,
    script_pats: Vec<(String, String)>,
    dom_pats: Vec<(String, String)>,
    js_pats: Vec<(String, String)>,
    headers_map: HashMap<String, Vec<(Option<String>, String)>>,
    cookies_map: HashMap<String, Vec<(Option<String>, String)>>,
    meta_map: HashMap<String, Vec<(Option<String>, String)>>,
    implies_map: HashMap<String, Vec<String>>,
}

fn parse_apps(apps: &serde_json::Map<String, Value>) -> ParsedApps {
    let mut html_pats: Vec<(String, String)> = Vec::new();
    let mut script_pats: Vec<(String, String)> = Vec::new();
    let mut dom_pats: Vec<(String, String)> = Vec::new();
    let mut js_pats: Vec<(String, String)> = Vec::new();
    let mut headers_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut cookies_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut meta_map: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut implies_map: HashMap<String, Vec<String>> = HashMap::new();

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
                        Value::Array(arr) => {
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

        if let Some(v) = obj.get("implies") {
            let implied: Vec<String> = value_as_strings(v)
                .into_iter()
                .map(|s| implies_name(&s).to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !implied.is_empty() {
                implies_map.insert(tech_name.clone(), implied);
            }
        }
    }

    ParsedApps {
        html_pats,
        script_pats,
        dom_pats,
        js_pats,
        headers_map,
        cookies_map,
        meta_map,
        implies_map,
    }
}

// ── PyO3 methods ──────────────────────────────────────────────────────────────

#[pymethods]
impl TechDetector {
    #[new]
    #[pyo3(signature = (json_data, chunk_size = 28))]
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
        let ParsedApps {
            html_pats,
            script_pats,
            dom_pats,
            js_pats,
            headers_map,
            cookies_map,
            meta_map,
            implies_map,
        } = parse_apps(apps);

        let html = build_text_matcher(html_pats, true, chunk_size, &mut skipped);
        let script = build_text_matcher(script_pats, true, chunk_size, &mut skipped);
        let dom = build_text_matcher(dom_pats, true, chunk_size, &mut skipped);
        let js = build_text_matcher(js_pats, false, chunk_size, &mut skipped);
        let headers = build_dict_matcher(headers_map, &mut skipped);
        let cookies = build_dict_matcher(cookies_map, &mut skipped);
        let meta = build_dict_matcher(meta_map, &mut skipped);
        let deps = DependencyResolver { implies: implies_map };

        Ok(TechDetector { html, script, dom, js, headers, cookies, meta, deps, skipped })
    }

    /// Full detection: body text + headers/cookies/meta + dependency resolution.
    ///
    /// - `html_parts`   — page bodies split on `</div>`, all pages merged
    /// - `script_parts` — script-tag contents split on blank lines, all pages merged
    /// - `headers`      — one dict per page
    /// - `cookies`      — one dict per page
    /// - `meta_tags`    — flat list of single-key dicts from all pages
    #[pyo3(signature = (html_parts, script_parts, headers, cookies, meta_tags, include_dependencies=true))]
    pub fn detect_full(
        &self,
        html_parts: Vec<Vec<u8>>,
        script_parts: Vec<Vec<u8>>,
        headers: Vec<HashMap<String, String>>,
        cookies: Vec<HashMap<String, String>>,
        meta_tags: Vec<HashMap<String, String>>,
        include_dependencies: bool,
    ) -> Vec<String> {
        let mut names: HashSet<String> = HashSet::new();

        names.extend(self.html.match_parts(&html_parts));
        names.extend(self.dom.match_parts(&html_parts));
        names.extend(self.script.match_parts(&script_parts));
        names.extend(self.js.match_parts(&script_parts));

        for h in &headers {
            names.extend(self.headers.match_dict(h));
        }
        for c in &cookies {
            names.extend(self.cookies.match_dict(c));
        }
        for m in &meta_tags {
            names.extend(self.meta.match_dict(m));
        }

        if include_dependencies {
            self.deps.resolve(&mut names);
        }

        names.into_iter().collect()
    }

    /// Resolve implies dependencies for an already-detected set of tech names.
    /// Useful when you need to filter detections before expanding dependencies.
    pub fn resolve_dependencies(&self, names: Vec<String>) -> Vec<String> {
        let mut set: HashSet<String> = names.into_iter().collect();
        self.deps.resolve(&mut set);
        set.into_iter().collect()
    }

    /// Low-level: match a single text key against a list of byte parts.
    pub fn detect_text_key(&self, key: &str, parts: Vec<Vec<u8>>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "html" => &self.html,
            "script" => &self.script,
            "dom" => &self.dom,
            "js" => &self.js,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown key: {other}"
                )))
            }
        };
        Ok(matcher.match_parts(&parts))
    }

    /// Low-level: match a single dict key against one Python dict.
    pub fn detect_dict_key(&self, key: &str, data: &Bound<'_, PyDict>) -> PyResult<Vec<String>> {
        let matcher = match key {
            "headers" => &self.headers,
            "cookies" => &self.cookies,
            "meta" => &self.meta,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown key: {other}"
                )))
            }
        };
        let map: HashMap<String, String> = data
            .iter()
            .filter_map(|(k, v)| {
                Some((k.extract::<String>().ok()?, v.extract::<String>().ok()?))
            })
            .collect();
        Ok(matcher.match_dict(&map))
    }

    pub fn pattern_counts(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        counts.insert("html".into(), self.html.total_patterns);
        counts.insert("script".into(), self.script.total_patterns);
        counts.insert("dom".into(), self.dom.total_patterns);
        counts.insert("js".into(), self.js.total_patterns);
        counts.insert(
            "headers".into(),
            self.headers.entries.values().map(|v| v.len()).sum(),
        );
        counts.insert(
            "cookies".into(),
            self.cookies.entries.values().map(|v| v.len()).sum(),
        );
        counts.insert(
            "meta".into(),
            self.meta.entries.values().map(|v| v.len()).sum(),
        );
        counts.insert("implies".into(), self.deps.implies.len());
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
                "js": {"React.version": "([0-9.]+)\\;version:\\1"},
                "implies": "Webpack"
            },
            "Webpack": {
                "script": "webpack"
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
        let names =
            d.detect_text_key("html", vec![b"<div class='wp-content'>".to_vec()]).unwrap();
        assert!(names.contains(&"WordPress".to_string()), "got {names:?}");
    }

    #[test]
    fn test_html_case_insensitive() {
        let d = detector();
        let names =
            d.detect_text_key("html", vec![b"React.createElement(App)".to_vec()]).unwrap();
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
    fn test_detect_full_with_dependencies() {
        let d = detector();
        // React implies Webpack — detect_full should return both
        let names = d.detect_full(
            vec![b"React.createElement(App)".to_vec()],
            vec![],
            vec![],
            vec![],
            vec![],
            true,
        );
        assert!(names.contains(&"React".to_string()), "expected React, got {names:?}");
        assert!(names.contains(&"Webpack".to_string()), "expected Webpack (implied), got {names:?}");
    }

    #[test]
    fn test_detect_full_no_dependencies() {
        let d = detector();
        let names = d.detect_full(
            vec![b"React.createElement(App)".to_vec()],
            vec![],
            vec![],
            vec![],
            vec![],
            false,
        );
        assert!(names.contains(&"React".to_string()));
        assert!(!names.contains(&"Webpack".to_string()), "Webpack should not appear without deps");
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

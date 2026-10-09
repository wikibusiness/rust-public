//! Replaces tldextract 5.3.2 and validators 0.35 `domain()` in platform3, and adds
//! the domain helpers of its `backend-python/common/utils/domain.py` on top.
//!
//! Every function reproduces the Python library it replaces exactly, including its
//! quirks: `tests/parity.py` fuzzes each one against the original.
//!
//! The Public Suffix List is embedded (`public_suffix_list.dat`) instead of fetched at
//! runtime like tldextract did. The refresh-domain_parsing_tools-psl workflow updates
//! it monthly and publishes a patch version when it changed.

use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;
use regex::{Regex, RegexBuilder};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

create_exception!(
    domain_parsing_tools,
    UnsupportedDomain,
    PyValueError,
    "The domain is not plain ASCII or has an xn-- label: use the Python implementation."
);

const PUBLIC_SUFFIX_LIST: &str = include_str!("../public_suffix_list.dat");

// tldextract's PUBLIC_SUFFIX_RE (re.MULTILINE).
static SUFFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[.*!]*\w\S*").unwrap());

static IPV4_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])\.){3}(?:[0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])$",
    )
    .unwrap()
});

// validators 0.35 `domain()` with its default flags (no rfc_1034, no rfc_2782).
static VALIDATORS_DOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r"^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9][a-z0-9-_]{0,61}[a-z]$",
    )
    .case_insensitive(true)
    .build()
    .unwrap()
});

// utils/domain.py's STOP_SUBDOMAIN_RE; Python's `$` also matches before a final "\n".
static STOP_SUBDOMAIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ww[w\d]\d?(?:\.|\n?\z)").unwrap());

struct SuffixList {
    public: Vec<&'static str>,
    private: Vec<&'static str>,
    incl_private: Trie,
    excl_private: Trie,
}

static SUFFIXES: LazyLock<SuffixList> = LazyLock::new(|| {
    let (public_text, private_text) = PUBLIC_SUFFIX_LIST
        .split_once("// ===BEGIN PRIVATE DOMAINS===")
        .unwrap_or((PUBLIC_SUFFIX_LIST, ""));
    let parse = |text: &'static str| SUFFIX_RE.find_iter(text).map(|m| m.as_str()).collect::<Vec<_>>();
    let (public, private) = (parse(public_text), parse(private_text));
    let mut incl_private = Trie::default();
    let mut excl_private = Trie::default();
    for suffix in &public {
        incl_private.add(suffix, false);
        excl_private.add(suffix, false);
    }
    for suffix in &private {
        incl_private.add(suffix, true);
    }
    SuffixList { public, private, incl_private, excl_private }
});

#[derive(Default)]
struct Node {
    children: HashMap<String, usize>,
    end: bool,
    is_private: bool,
}

/// tldextract's `Trie`: suffix labels in reverse order.
struct Trie {
    nodes: Vec<Node>,
}

impl Default for Trie {
    fn default() -> Self {
        Self { nodes: vec![Node::default()] }
    }
}

impl Trie {
    fn add(&mut self, suffix: &str, is_private: bool) {
        let mut node = 0;
        for label in suffix.rsplit('.') {
            node = match self.nodes[node].children.get(label) {
                Some(&child) => child,
                None => {
                    self.nodes.push(Node::default());
                    let child = self.nodes.len() - 1;
                    self.nodes[node].children.insert(label.to_string(), child);
                    child
                }
            };
        }
        self.nodes[node].end = true;
        self.nodes[node].is_private = is_private;
    }

    /// `_PublicSuffixListTLDExtractor.suffix_index`: the index of the first public
    /// suffix label with its node's `is_private`, and the first registry suffix label.
    fn suffix_index(&self, labels: &[&str]) -> Option<((usize, bool), usize)> {
        let mut node = 0;
        let mut label_idx = labels.len();
        let mut suffix_idx = labels.len();
        let mut reg_idx = labels.len();
        for label in labels.iter().rev() {
            let decoded = decode_punycode(label);
            let children = &self.nodes[node].children;
            if let Some(&child) = children.get(decoded.as_ref()) {
                label_idx -= 1;
                node = child;
                if self.nodes[node].end {
                    suffix_idx = label_idx;
                    if !self.nodes[node].is_private {
                        reg_idx = label_idx;
                    }
                }
                continue;
            }
            if let Some(&wildcard) = children.get("*") {
                let is_exception = children.contains_key(&format!("!{decoded}"));
                let idx = if is_exception { label_idx } else { label_idx - 1 };
                return Some(((idx, self.nodes[wildcard].is_private), reg_idx));
            }
            break;
        }
        (suffix_idx != labels.len()).then_some(((suffix_idx, self.nodes[node].is_private), reg_idx))
    }
}

/// tldextract's `_decode_punycode`. Skips idna's `check_label` on the decoded label:
/// it only feeds suffix lookups, and every non-ASCII suffix passes it (asserted in
/// tests/parity.py), so the lookup result is the same.
fn decode_punycode(label: &str) -> Cow<'_, str> {
    let lowered = label.to_lowercase();
    if let Some(rest) = lowered.strip_prefix("xn--") {
        let decodable = lowered.is_ascii() && lowered.len() <= 254 && !rest.is_empty() && !rest.ends_with('-');
        if decodable {
            if let Some(decoded) = idna::punycode::decode_to_string(rest) {
                if idna::punycode::encode_str(&decoded).as_deref() == Some(rest) {
                    return Cow::Owned(decoded);
                }
            }
        }
    }
    Cow::Owned(lowered)
}

/// Python's `str.isspace()`: Unicode White_Space plus the ASCII separators.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// tldextract's `lenient_netloc`.
fn lenient_netloc(url: &str) -> &str {
    let schemeless = match url.find("//") {
        Some(0) => &url[2..],
        Some(i)
            if i >= 2
                && url.as_bytes()[i - 1] == b':'
                && url[..i - 1].bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) =>
        {
            &url[i + 2..]
        }
        _ => url,
    };
    let before_path = schemeless.split(['/', '?', '#']).next().unwrap_or("");
    let after_userinfo = before_path.rsplit('@').next().unwrap_or("");
    if after_userinfo.starts_with('[') {
        if let Some(end) = after_userinfo.find(']') {
            return &after_userinfo[..=end];
        }
    }
    let hostname = after_userinfo.split(':').next().unwrap_or("").trim_matches(is_py_space);
    hostname.trim_end_matches(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}'])
}

#[pyclass(frozen, get_all, from_py_object, module = "domain_parsing_tools")]
#[derive(Clone, Debug, PartialEq)]
struct ExtractResult {
    subdomain: String,
    domain: String,
    suffix: String,
    is_private: bool,
    registry_suffix: String,
}

#[pymethods]
impl ExtractResult {
    #[getter]
    fn registered_domain(&self) -> String {
        if self.suffix.is_empty() || self.domain.is_empty() {
            return String::new();
        }
        format!("{}.{}", self.domain, self.suffix)
    }

    #[getter]
    fn top_domain_under_public_suffix(&self) -> String {
        self.registered_domain()
    }

    #[getter]
    fn fqdn(&self) -> String {
        if self.suffix.is_empty() || (self.domain.is_empty() && !self.is_private) {
            return String::new();
        }
        [&self.subdomain, &self.domain, &self.suffix]
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    fn __repr__(&self) -> String {
        format!("{self:?}")
    }
}

fn host_only(netloc: String) -> ExtractResult {
    ExtractResult {
        subdomain: String::new(),
        domain: netloc,
        suffix: String::new(),
        is_private: false,
        registry_suffix: String::new(),
    }
}

/// tldextract's `TLDExtract.extract_str`.
fn extract_url(url: &str, include_private: bool, looks_like_ipv6: &dyn Fn(&str) -> bool) -> ExtractResult {
    let netloc = lenient_netloc(url).replace(['\u{3002}', '\u{ff0e}', '\u{ff61}'], ".");
    if netloc.chars().count() >= 4
        && netloc.starts_with('[')
        && netloc.ends_with(']')
        && looks_like_ipv6(&netloc[1..netloc.len() - 1])
    {
        return host_only(netloc);
    }
    let labels: Vec<&str> = netloc.split('.').collect();
    let suffixes = &*SUFFIXES;
    let trie = if include_private { &suffixes.incl_private } else { &suffixes.excl_private };
    let Some(((suffix_idx, is_private), reg_idx)) = trie.suffix_index(&labels) else {
        if labels.len() == 4 && IPV4_RE.is_match(&netloc) {
            return host_only(netloc);
        }
        return ExtractResult {
            subdomain: labels[..labels.len() - 1].join("."),
            domain: labels[labels.len() - 1].to_string(),
            suffix: String::new(),
            is_private: false,
            registry_suffix: String::new(),
        };
    };
    let suffix = labels[suffix_idx..].join(".");
    ExtractResult {
        subdomain: if suffix_idx >= 2 { labels[..suffix_idx - 1].join(".") } else { String::new() },
        domain: if suffix_idx > 0 { labels[suffix_idx - 1].to_string() } else { String::new() },
        registry_suffix: if is_private { labels[reg_idx..].join(".") } else { suffix.clone() },
        suffix,
        is_private,
    }
}

/// utils/domain.py's `custom_tldextract`: tldextract with PSL private domains, and
/// gov.uk-style suffixes where the "domain" is a www label or there is no
/// registrable domain move the suffix's first label into the domain.
fn custom_extract_url(url: &str, looks_like_ipv6: &dyn Fn(&str) -> bool) -> ExtractResult {
    let result = extract_url(url, true, looks_like_ipv6);
    let is_stop_subdomain = STOP_SUBDOMAIN_RE.is_match(&result.domain);
    let Some((domain, suffix)) = result.suffix.split_once('.') else {
        return result;
    };
    if !is_stop_subdomain && !result.registered_domain().is_empty() {
        return result;
    }
    ExtractResult {
        subdomain: if is_stop_subdomain { result.domain.clone() } else { result.subdomain.clone() },
        domain: domain.to_string(),
        suffix: suffix.to_string(),
        is_private: result.is_private,
        registry_suffix: suffix.to_string(),
    }
}

/// validators 0.35 `domain(value)`. `idna2003` is Python's `str.encode("idna")`,
/// only called for non-ASCII values.
fn check_domain(value: &str, idna2003: &dyn Fn(&str) -> Option<String>) -> bool {
    if value.is_empty() || value.contains("__") || value.chars().any(is_py_space) {
        return false;
    }
    let encoded = if value.is_ascii() {
        // encodings.idna's ASCII fast path.
        let labels: Vec<&str> = value.split('.').collect();
        if labels[..labels.len() - 1].iter().any(|label| label.is_empty()) || labels.iter().any(|label| label.len() >= 64) {
            return false;
        }
        Cow::Borrowed(value)
    } else {
        match idna2003(value) {
            Some(encoded) => Cow::Owned(encoded),
            None => return false,
        }
    };
    VALIDATORS_DOMAIN_RE.is_match(&encoded)
}

fn is_alabel(label: &str) -> bool {
    label.as_bytes().get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"xn--"))
}

/// idna 3.x `check_label` on a lowercased ASCII non-A-label: PVALID in ASCII is
/// exactly `[a-z0-9-]`, and no ASCII codepoint is CONTEXTJ/CONTEXTO or RTL.
fn is_valid_ascii_ulabel(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && bytes.get(2..4) != Some(b"--")
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

fn decode_idna_ascii(domain: &str) -> Option<Option<String>> {
    if domain.contains('_') {
        return Some(Some(domain.to_string()));
    }
    if !domain.is_ascii() {
        return None;
    }
    let mut labels: Vec<&str> = domain.split('.').collect();
    if labels.iter().any(|label| is_alabel(label)) {
        return None;
    }
    if domain.is_empty() || domain.len() > 254 {
        return Some(None);
    }
    if labels.last() == Some(&"") {
        labels.pop();
    }
    if !labels.iter().all(|label| is_valid_ascii_ulabel(label)) {
        return Some(None);
    }
    Some(Some(domain.to_ascii_lowercase()))
}

/// `idna.encode(domain)` (idna 3.x defaults: strict IDNA 2008, no UTS 46 mapping) on
/// a plain-ASCII domain, `Some(None)` where it raises. `None` (unsupported) for
/// non-ASCII domains, or once an A-label is reached: those need the Python codec.
fn idna_encode_ascii(domain: &str) -> Option<Option<String>> {
    if !domain.is_ascii() {
        return None;
    }
    let body = domain.strip_suffix('.');
    let max_len = if body.is_some() { 254 } else { 253 };
    if domain.len() > max_len {
        return Some(None);
    }
    // Labels are checked in order: idna raises at the first invalid one, so an
    // invalid label before any A-label is a rejection whatever the A-label holds.
    for label in body.unwrap_or(domain).split('.') {
        if is_alabel(label) {
            return None;
        }
        if label.len() > 63 || !is_valid_ascii_ulabel(label) {
            return Some(None);
        }
    }
    Some(Some(domain.to_string()))
}

fn py_looks_like_ipv6(py: Python<'_>) -> impl Fn(&str) -> bool + '_ {
    move |value| {
        py.import("ipaddress")
            .and_then(|module| module.getattr("IPv6Address")?.call1((value,)))
            .is_ok()
    }
}

fn py_idna2003(py: Python<'_>) -> impl Fn(&str) -> Option<String> + '_ {
    move |value| {
        let encoded = PyString::new(py, value).call_method1("encode", ("idna",)).ok()?;
        String::from_utf8(encoded.extract::<Vec<u8>>().ok()?).ok()
    }
}

/// Surrogates can't be a Rust `str`: replaced with U+FFFD instead of raising.
fn lossy<'a>(value: &'a Bound<'_, PyString>) -> Cow<'a, str> {
    value.to_string_lossy()
}

/// `tldextract.TLDExtract(include_psl_private_domains=...)(url)`.
#[pyfunction]
#[pyo3(signature = (url, include_psl_private_domains = false))]
fn extract(py: Python<'_>, url: &Bound<'_, PyString>, include_psl_private_domains: bool) -> ExtractResult {
    extract_url(&lossy(url), include_psl_private_domains, &py_looks_like_ipv6(py))
}

/// `custom_tldextract` from utils/domain.py.
#[pyfunction]
fn custom_extract(py: Python<'_>, url: &Bound<'_, PyString>) -> ExtractResult {
    custom_extract_url(&lossy(url), &py_looks_like_ipv6(py))
}

/// Suffixes only in the PSL's private section, i.e. tldextract's
/// `set(TLDExtract(include_psl_private_domains=True).tlds) - set(TLDExtract().tlds)`.
#[pyfunction]
fn private_suffixes() -> HashSet<&'static str> {
    let suffixes = &*SUFFIXES;
    let public: HashSet<&str> = suffixes.public.iter().copied().collect();
    suffixes.private.iter().copied().filter(|suffix| !public.contains(suffix)).collect()
}

/// `bool(validators.domain(value))`: `False` for any non-str, like validators.
#[pyfunction]
fn is_domain(py: Python<'_>, value: &Bound<'_, PyAny>) -> bool {
    let Ok(value) = value.cast::<PyString>() else {
        return false;
    };
    check_domain(&lossy(value), &py_idna2003(py))
}

/// `is_valid_main_domain` from utils/domain.py: `False` for any non-str.
#[pyfunction]
fn is_valid_main_domain(py: Python<'_>, domain: &Bound<'_, PyAny>) -> bool {
    let Ok(domain) = domain.cast::<PyString>() else {
        return false;
    };
    let domain = lossy(domain);
    check_domain(&domain, &py_idna2003(py))
        && domain == custom_extract_url(&domain, &py_looks_like_ipv6(py)).registered_domain()
}

/// `decode_idna` from utils/domain.py (`idna.decode(domain)`, `None` when it raises,
/// domains containing `_` returned untouched). Raises `UnsupportedDomain` outside
/// plain-ASCII domains with no A-label: the caller falls back to Python's idna.
#[pyfunction]
fn decode_idna(domain: &Bound<'_, PyString>) -> PyResult<Option<String>> {
    let domain = domain.to_str().map_err(|_| UnsupportedDomain::new_err("not valid UTF-8"))?;
    decode_idna_ascii(domain).ok_or_else(|| UnsupportedDomain::new_err("outside the ASCII fast path"))
}

/// `idna.encode(domain).decode()`, `None` when it raises `IDNAError`. Raises
/// `UnsupportedDomain` for non-ASCII domains or ones reaching an A-label: the caller
/// falls back to Python's idna.
#[pyfunction]
fn idna_encode(domain: &Bound<'_, PyString>) -> PyResult<Option<String>> {
    let domain = domain.to_str().map_err(|_| UnsupportedDomain::new_err("not valid UTF-8"))?;
    idna_encode_ascii(domain).ok_or_else(|| UnsupportedDomain::new_err("outside the ASCII fast path"))
}

#[pymodule]
fn domain_parsing_tools(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(extract, m)?)?;
    m.add_function(wrap_pyfunction!(custom_extract, m)?)?;
    m.add_function(wrap_pyfunction!(private_suffixes, m)?)?;
    m.add_function(wrap_pyfunction!(is_domain, m)?)?;
    m.add_function(wrap_pyfunction!(is_valid_main_domain, m)?)?;
    m.add_function(wrap_pyfunction!(decode_idna, m)?)?;
    m.add_function(wrap_pyfunction!(idna_encode, m)?)?;
    m.add_class::<ExtractResult>()?;
    m.add("UnsupportedDomain", m.py().get_type::<UnsupportedDomain>())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(url: &str, include_private: bool) -> (String, String, String) {
        let r = extract_url(url, include_private, &|_| false);
        (r.subdomain, r.domain, r.suffix)
    }

    fn owned(s: (&str, &str, &str)) -> (String, String, String) {
        (s.0.into(), s.1.into(), s.2.into())
    }

    #[test]
    fn extract_matches_tldextract() {
        assert_eq!(parts("http://forums.news.cnn.com/", false), owned(("forums.news", "cnn", "com")));
        assert_eq!(parts("https://user@Forums.BBC.co.uk:8080/x?y#z", false), owned(("Forums", "BBC", "co.uk")));
        assert_eq!(parts("waiterrant.blogspot.com", false), owned(("waiterrant", "blogspot", "com")));
        assert_eq!(parts("waiterrant.blogspot.com", true), owned(("", "waiterrant", "blogspot.com")));
        assert_eq!(parts("xn--mnchen-3ya.xn--p1ai", false), owned(("", "xn--mnchen-3ya", "xn--p1ai")));
        assert_eq!(parts("localhost", false), owned(("", "localhost", "")));
        assert_eq!(parts("127.0.0.1", false), owned(("", "127.0.0.1", "")));
        assert_eq!(parts("acme.com.", false), owned(("", "acme", "com")));
    }

    #[test]
    fn custom_extract_moves_gov_uk() {
        let r = custom_extract_url("www.gov.uk", &|_| false);
        assert_eq!((r.subdomain.as_str(), r.domain.as_str(), r.suffix.as_str()), ("www", "gov", "uk"));
    }

    #[test]
    fn check_domain_matches_validators() {
        let no_idna = |_: &str| None;
        assert!(check_domain("acme.com", &no_idna));
        assert!(!check_domain("-acme.com", &no_idna));
        assert!(!check_domain("a__b.com", &no_idna));
        assert!(!check_domain("a b.com", &no_idna));
        assert!(!check_domain("acme", &no_idna));
    }

    #[test]
    fn decode_idna_ascii_matches_idna() {
        assert_eq!(decode_idna_ascii("Acme.COM"), Some(Some("acme.com".into())));
        assert_eq!(decode_idna_ascii("acme.com."), Some(Some("acme.com.".into())));
        assert_eq!(decode_idna_ascii("a_b.com"), Some(Some("a_b.com".into())));
        for invalid in ["", ".", "a..com", "-a.com", "a-.com", "ab--c.com", "a b.com"] {
            assert_eq!(decode_idna_ascii(invalid), Some(None), "{invalid}");
        }
        assert_eq!(decode_idna_ascii("xn--bcher-kva.de"), None);
        assert_eq!(decode_idna_ascii("bücher.de"), None);
    }

    #[test]
    fn idna_encode_ascii_matches_idna() {
        let label63 = "a".repeat(63);
        let label64 = "a".repeat(64);
        let long = [label63.as_str(); 4].join(".");
        let (d253, d254) = (&long[..253], &long[..254]);
        let valid = [
            "acme.com", "ACME.Com", "123.com", "a-b.com", "a--b.com", "a.b.c.d", "acme.com.",
            label63.as_str(), d253, &format!("{d253}."),
        ];
        for domain in valid {
            assert_eq!(idna_encode_ascii(domain), Some(Some(domain.to_string())), "{domain}");
        }
        let invalid = [
            "", ".", "..", ".acme.com", "a..com", "acme.com..", "-a.com", "a-.com", "ab--c.com",
            "my_site.com", "a b.com", "a@b.com", "a*.com", label64.as_str(), d254, &format!("{d254}."),
            // An invalid label before an A-label is rejected without decoding it.
            "-a.xn--bcher-kva.de",
        ];
        for domain in invalid {
            assert_eq!(idna_encode_ascii(domain), Some(None), "{domain}");
        }
        for unsupported in ["xn--bcher-kva.de", "XN--bcher-kva.de", "a.xn--zz.de", "xn--.de", "bücher.de", "é.com", "a\u{200d}b.com"] {
            assert_eq!(idna_encode_ascii(unsupported), None, "{unsupported}");
        }
    }
}

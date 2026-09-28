//! Fast paths for platform3's `utils/domain.py` (`decode_idna`, `is_valid_main_domain`).
//!
//! Both only cover plain-ASCII domains with no A-label (`xn--`): anything else raises
//! `UnsupportedDomain` and the caller falls back to the Python implementation, which
//! owns the full IDNA 2008 tables. Within that scope the result must equal the Python
//! one exactly; platform3's `test_domain.py` fuzzes the two against each other.

use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use regex::{Regex, RegexBuilder};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

create_exception!(
    domain_parsing_tools,
    UnsupportedDomain,
    PyValueError,
    "The domain is not plain ASCII or has an xn-- label: use the Python implementation."
);

// validators 0.35 `domain()` with its default flags (no rfc_1034, no rfc_2782).
static VALIDATORS_DOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r"^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9][a-z0-9-_]{0,61}[a-z]$",
    )
    .case_insensitive(true)
    .build()
    .unwrap()
});

// utils/domain.py's STOP_SUBDOMAIN_RE, applied to a label (never contains a dot).
static STOP_SUBDOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ww[w0-9][0-9]?$").unwrap());

fn is_alabel(label: &str) -> bool {
    label.as_bytes().get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"xn--"))
}

fn unsupported() -> PyErr {
    UnsupportedDomain::new_err("outside the ASCII fast path")
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

/// `decode_idna` from utils/domain.py: `idna.decode(domain)`, `None` when it raises,
/// and domains containing `_` returned untouched.
#[pyfunction]
fn decode_idna(domain: &str) -> PyResult<Option<String>> {
    decode_idna_ascii(domain).ok_or_else(unsupported)
}

#[derive(Default)]
struct Node {
    children: HashMap<String, usize>,
    end: bool,
}

/// tldextract's suffix trie, built from the same suffix set tldextract loaded.
#[pyclass(frozen, module = "domain_parsing_tools")]
struct PublicSuffixList {
    nodes: Vec<Node>,
}

impl PublicSuffixList {
    fn from_suffixes<'a>(suffixes: impl IntoIterator<Item = &'a str>) -> Self {
        let mut nodes = vec![Node::default()];
        for suffix in suffixes {
            let mut node = 0;
            for label in suffix.rsplit('.') {
                node = match nodes[node].children.get(label) {
                    Some(&child) => child,
                    None => {
                        nodes.push(Node::default());
                        let child = nodes.len() - 1;
                        nodes[node].children.insert(label.to_string(), child);
                        child
                    }
                };
            }
            nodes[node].end = true;
        }
        Self { nodes }
    }

    /// tldextract's `_PublicSuffixListTLDExtractor.suffix_index`: index of the first
    /// public suffix label, or `None` when no suffix matches.
    fn suffix_index(&self, labels: &[&str]) -> Option<usize> {
        let mut node = 0;
        let mut label_idx = labels.len();
        let mut suffix_idx = labels.len();
        for label in labels.iter().rev() {
            let lowered = label.to_ascii_lowercase();
            if let Some(&child) = self.nodes[node].children.get(&lowered) {
                label_idx -= 1;
                node = child;
                if self.nodes[node].end {
                    suffix_idx = label_idx;
                }
                continue;
            }
            let children = &self.nodes[node].children;
            if children.contains_key("*") {
                let is_exception = children.contains_key(&format!("!{lowered}"));
                return Some(if is_exception { label_idx } else { label_idx - 1 });
            }
            break;
        }
        (suffix_idx != labels.len()).then_some(suffix_idx)
    }

    /// `None` when outside the fast path.
    fn check_main_domain(&self, domain: &str) -> Option<bool> {
        if !domain.is_ascii() {
            return None;
        }
        if domain.contains("__") || !VALIDATORS_DOMAIN_RE.is_match(domain) {
            return Some(false);
        }
        let labels: Vec<&str> = domain.split('.').collect();
        if labels.iter().any(|label| is_alabel(label)) {
            return None;
        }
        let Some(suffix_idx) = self.suffix_index(&labels) else {
            return Some(false);
        };
        let mut name = if suffix_idx > 0 { labels[suffix_idx - 1] } else { "" };
        let mut suffix = &labels[suffix_idx..];
        // custom_tldextract: gov.uk-style suffixes where the "domain" is a www label or
        // there is no registrable domain move the suffix's first label into the domain.
        if (STOP_SUBDOMAIN_RE.is_match(name) || name.is_empty()) && suffix.len() >= 2 {
            name = suffix[0];
            suffix = &suffix[1..];
        }
        if name.is_empty() || suffix.is_empty() {
            return Some(false);
        }
        Some(domain == format!("{name}.{}", suffix.join(".")))
    }
}

#[pymethods]
impl PublicSuffixList {
    #[new]
    fn new(suffixes: HashSet<String>) -> Self {
        Self::from_suffixes(suffixes.iter().map(String::as_str))
    }

    /// `is_valid_main_domain` from utils/domain.py (validators.domain plus the
    /// custom_tldextract registered-domain equality).
    fn is_valid_main_domain(&self, domain: &str) -> PyResult<bool> {
        self.check_main_domain(domain).ok_or_else(unsupported)
    }
}

#[pymodule]
fn domain_parsing_tools(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(decode_idna, m)?)?;
    m.add_class::<PublicSuffixList>()?;
    m.add("UnsupportedDomain", m.py().get_type::<UnsupportedDomain>())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn main_domain_follows_tldextract_and_custom_rules() {
        let psl = PublicSuffixList::from_suffixes(["com", "uk", "gov.uk", "co.uk", "*.ck", "!www.ck"]);
        assert_eq!(psl.check_main_domain("acme.com"), Some(true));
        assert_eq!(psl.check_main_domain("Acme.co.uk"), Some(true));
        assert_eq!(psl.check_main_domain("www.acme.com"), Some(false));
        assert_eq!(psl.check_main_domain("gov.uk"), Some(true));
        assert_eq!(psl.check_main_domain("www.gov.uk"), Some(false));
        assert_eq!(psl.check_main_domain("acme.ck"), Some(true));
        assert_eq!(psl.check_main_domain("www.ck"), Some(true));
        assert_eq!(psl.check_main_domain("acme.org"), Some(false));
        assert_eq!(psl.check_main_domain("xn--bcher-kva.com"), None);
    }
}

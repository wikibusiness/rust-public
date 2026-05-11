// DOM selector → regex conversion — mirrors oceanai/utils/technologies.py dom_to_regex()

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

const XPATH_TAGS: &[&str] = &[
    "a", "amp-img", "body", "button", "clipPath", "div", "embed", "form", "html", "iframe",
    "img", "input", "link", "meta", "noscript", "object", "param", "script", "style", "section",
    "source", "title", "track", "video",
];

const XPATH_PROPS: &[&str] = &[
    "action",
    "class",
    "data-abuse-proto",
    "data-animation-type",
    "data-bg",
    "data-function",
    "data-hid",
    "data-lazy-src",
    "data-meta",
    "data-name",
    "data-quadpay-src",
    "data-page",
    "data-src",
    "data-srcset",
    "data-requiremodule",
    "data-testid",
    "data-video-provider",
    "href",
    "id",
    "imagesrcset",
    "name",
    "poster",
    "scr",
    "src",
    "srcset",
    "style",
    "title",
    "type",
    "value",
];

const DOM_STOPWORDS: &[&str] = &["browser", "footer", "print-only", ">"];

// Matches: tag[prop*='value'] or tag[prop^='value'] or tag[prop='value']
static DOM_RE: Lazy<Regex> = Lazy::new(|| {
    let tags = XPATH_TAGS.join("|");
    let props = XPATH_PROPS.join("|");
    Regex::new(&format!(
        r"({tags})\[({props})[\*\^]?\='?(.*?)'?\]"
    ))
    .unwrap()
});

// Matches: tag[prop]  (presence-only attribute check)
static DOM2_RE: Lazy<Regex> = Lazy::new(|| {
    let tags = XPATH_TAGS.join("|");
    Regex::new(&format!(r"({tags})\[([a-zA-Z\-]+)\]")).unwrap()
});

fn get_dom_1(tag: &str) -> String {
    format!("<(?:[^>]*?){tag}")
}

fn get_dom_2(tag: &str, prop: &str) -> String {
    format!("<(?:[^>]*?){tag}(.*?){prop}")
}

fn get_dom_3(tag: &str, prop: &str, value: &str) -> String {
    format!(r#"<{tag}\s+(?:[^>]*?\s+)?{prop}=(["'])(.*?){value}(.*?)"#)
}

fn dom_list_to_regex(doms: &[&str]) -> Vec<String> {
    let mut regexes = Vec::new();

    for dom_list in doms {
        for dom in dom_list.split(", ") {
            let dom = dom.trim();

            // tag[prop=value] matches
            for cap in DOM_RE.captures_iter(dom) {
                let prop = cap.get(2).map_or("", |m| m.as_str());
                let value = cap.get(3).map_or("", |m| m.as_str());
                if prop == "hidden" || value == "hidden" {
                    continue;
                }
                let tag = cap.get(1).map_or("", |m| m.as_str());
                regexes.push(get_dom_3(tag, prop, &value.replace('.', "\\.")));
            }

            // tag[prop] presence matches
            for cap in DOM2_RE.captures_iter(dom) {
                let tag = cap.get(1).map_or("", |m| m.as_str());
                let prop = cap.get(2).map_or("", |m| m.as_str());
                regexes.push(get_dom_2(tag, prop));
            }

            if dom.starts_with('.') {
                // .className → treat class part as tag name
                if let Some(class) = dom.splitn(2, '.').nth(1) {
                    regexes.push(get_dom_1(class));
                }
            } else if dom.starts_with("div.") {
                if let Some(class) = dom.splitn(2, '.').nth(1) {
                    regexes.push(get_dom_2("div", class));
                }
            } else if dom.contains('#') {
                let parts: Vec<&str> = dom.splitn(2, '#').collect();
                if parts.len() == 2 {
                    if parts[0].is_empty() {
                        regexes.push(get_dom_1(parts[1]));
                    } else {
                        regexes.push(get_dom_2(parts[0], &parts[1].replace('\'', "\\'")));
                    }
                }
            } else if dom.starts_with('[') && dom.ends_with(']') {
                let inner = &dom[1..dom.len() - 1];
                if dom.contains('=') {
                    // normalise ^= and *= to =
                    let normalised = inner.replace("^=", "=").replace("*=", "=");
                    let kv: Vec<&str> = normalised.splitn(2, '=').collect();
                    if kv.len() == 2 {
                        regexes.push(get_dom_2(kv[0], kv[1]));
                    }
                } else {
                    regexes.push(get_dom_1(inner));
                }
            }
        }
    }

    regexes
}

fn dom_dict_to_regex(doms: &serde_json::Map<String, Value>) -> (Vec<String>, Vec<String>) {
    let mut regexes = Vec::new();
    let mut doms_list = Vec::new();

    for (keys, item) in doms {
        if keys.ends_with('*') || DOM_STOPWORDS.iter().any(|sw| keys.contains(sw)) {
            continue;
        }

        for key in keys.split(',') {
            let key = key.trim();

            if XPATH_TAGS.contains(&key) {
                if let Value::Object(obj) = item {
                    if let Some(attrs) = obj.get("attributes").and_then(|a| a.as_object()) {
                        for (attr_key, attr_val) in attrs {
                            let attr_val = attr_val.as_str().unwrap_or("").replace('.', "\\.");
                            regexes.push(get_dom_3(key, attr_key, &attr_val));
                        }
                    } else if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                        regexes.push(get_dom_3(key, "text", &text.replace('.', "\\.")));
                    }
                }
            } else {
                doms_list.push(key.to_string());
            }
        }
    }

    (regexes, doms_list)
}

/// Convert a `dom` field value (string, list, or dict) into regex strings.
pub fn dom_to_regex(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => {
            dom_list_to_regex(&[s.as_str()])
        }
        Value::Array(arr) => {
            let strs: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
            dom_list_to_regex(&strs)
        }
        Value::Object(obj) => {
            let (mut regexes, list_items) = dom_dict_to_regex(obj);
            let strs: Vec<&str> = list_items.iter().map(|s| s.as_str()).collect();
            regexes.extend(dom_list_to_regex(&strs));
            regexes
        }
        _ => vec![],
    }
}

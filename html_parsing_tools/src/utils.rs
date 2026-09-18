use kuchikiki::iter::NodeIterator;
use kuchikiki::NodeRef;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::{HashMap, HashSet};

lazy_static! {
    static ref DESCRIPTION_ATTR_NAMES: HashSet<&'static str> =
        HashSet::from(["description", "og:description"]);
}

pub fn get_lang_internal(document: &NodeRef) -> String {
    let tag_nodes = document.select("html").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes = tag_node.attributes.borrow();
        let type_attribute = attributes.get("lang").unwrap_or("");
        return type_attribute.to_string();
    }
    return "".to_string();
}

pub fn apply(sentences: Vec<String>, stop_word_regex: &Regex) -> Vec<String> {
    sentences
        .iter()
        .map(|n| stop_word_regex.replace_all(&n, "").trim().to_string())
        .map(|n| n.split_whitespace().collect::<Vec<&str>>().join(" "))
        .filter(|n| !n.to_lowercase().contains("cookie") && !n.contains("©") && count_words(n) > 0)
        .collect()
}

pub fn get_json_ld(document: &NodeRef) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let tag_nodes = document.select("script").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes = tag_node.attributes.borrow();
        let type_attribute = attributes.get("type").unwrap_or("");
        if type_attribute == "application/ld+json" {
            result.push(get_text_string(tag_node.as_node(), " "));
        }
    }
    return result;
}

pub fn get_rel_alternate(document: &NodeRef) -> HashMap<String, Vec<String>> {
    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    let tag_nodes = document.select("link").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes = tag_node.attributes.borrow();
        let type_attribute = attributes.get("rel").unwrap_or("");
        if type_attribute == "alternate" {
            let hreflang = attributes.get("hreflang").unwrap_or("").to_string();
            if hreflang.is_empty() {
                continue;
            }
            if result.contains_key(hreflang.as_str()) {
                let mut new_data = result.get(hreflang.as_str()).unwrap().clone();
                let new_array = vec![attributes.get("href").unwrap_or("").to_string()];
                new_data.extend(new_array);
                result.insert(hreflang, new_data.to_owned());
            } else {
                result.insert(
                    hreflang,
                    vec![attributes.get("href").unwrap_or("").to_string()],
                );
            }
        }
    }
    return result;
}

/// Shared by the standalone get_meta_titles pyfunction and get_sentences,
/// so a caller that already has a GetSentencesResult for a page doesn't need
/// to reparse its HTML just to read these.
pub fn get_meta_titles_internal(document: &NodeRef) -> HashMap<String, String> {
    let mut result: HashMap<String, String> = HashMap::new();
    let tag_nodes = document.select("meta").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes = tag_node.attributes.borrow();
        let name_attribute = attributes.get("name").unwrap_or("");
        if name_attribute == "twitter:title" || name_attribute == "og:title" {
            let content = attributes.get("content").unwrap_or("").to_string();
            if content.is_empty() {
                continue;
            }
            result.insert(name_attribute.to_string(), content);
        }
    }
    let tag_nodes = document.select("title").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        result.insert(
            "title".to_string(),
            get_text_string(tag_node.as_node(), " "),
        );
    }
    result
}

/// Shared by the standalone get_href_attributes pyfunction and get_sentences
/// -- see get_meta_titles_internal.
pub fn get_href_attributes_internal(document: &NodeRef) -> Vec<String> {
    document
        .select("a")
        .unwrap()
        .map(|x| {
            let attributes = x.attributes.borrow();
            attributes.get("href").unwrap_or("").to_string()
        })
        .collect()
}

pub fn get_descriptions(document: &NodeRef) -> Vec<String> {
    let mut descriptions: HashSet<String> = HashSet::new();
    if let Ok(tag_nodes) = document.select("meta") {
        for tag_node in tag_nodes {
            let attributes = tag_node.attributes.borrow();
            let attribute_values: HashSet<&str> = HashSet::from([
                attributes.get("name").unwrap_or(""),
                attributes.get("property").unwrap_or(""),
            ]);

            if !DESCRIPTION_ATTR_NAMES.is_disjoint(&attribute_values) {
                if let Some(content_attribute) = attributes.get("content") {
                    if !content_attribute.is_empty() {
                        descriptions.insert(content_attribute.to_string());
                    }
                }
            }
        }
    }
    let mut output: Vec<String> = descriptions.into_iter().collect();
    output.sort();
    output
}

pub fn get_keywords(document: &NodeRef) -> Option<String> {
    let tag_nodes = document.select("meta").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes = tag_node.attributes.borrow();
        let name_attribute = attributes.get("name").unwrap_or("");
        if name_attribute == "keywords" {
            let content_attribute = attributes.get("content").unwrap_or("");
            if !content_attribute.is_empty() {
                return Some(content_attribute.to_string());
            }
        }
    }
    return None;
}

pub fn get_text_and_remove(document: &NodeRef, tag: &str) -> Vec<String> {
    let mut result: Vec<String> = vec![];
    let tag_nodes = document.select(tag).unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let text = trim_whitespace(get_text_string(tag_node.as_node(), " ").as_str());
        if !text.is_empty() {
            result.push(trim_whitespace(
                get_text_string(tag_node.as_node(), " ").as_str(),
            ));
        }
        let as_node = tag_node.as_node();
        as_node.detach();
    }
    return result;
}

pub fn get_text_nodes(node: &NodeRef) -> Vec<String> {
    node.inclusive_descendants()
        .text_nodes()
        .map(|text_node| text_node.borrow().trim().to_string())
        .filter(|string| {
            !string.is_empty()
                && !(string.contains("<")
                    && string.contains(">")
                    && (string.contains("</") || string.contains("/>")))
        })
        .collect()
}

pub fn get_text_string(node: &NodeRef, separator: &str) -> String {
    get_text_nodes(node).join(separator)
}

/// lxml's Cleaner(remove_tags=...) doesn't just reparent an unwrapped
/// element's children as separate siblings -- it merges runs of adjacent
/// text into one string (verified against the real Python output, not
/// assumed). unwrap_tag alone leaves them as separate sibling text nodes;
/// call this afterward to match, since form_text_nodes' grouping treats
/// separate items differently from one merged one, especially at
/// min_split=0 where nothing downstream re-merges them.
pub fn merge_adjacent_text_nodes(document: &NodeRef) {
    for node in document.inclusive_descendants() {
        let mut child = node.first_child();
        while let Some(current) = child {
            let next = current.next_sibling();
            if let (Some(cur_text), Some(next_node)) = (current.as_text(), next.clone()) {
                if let Some(next_text) = next_node.as_text() {
                    let appended = next_text.borrow().clone();
                    cur_text.borrow_mut().push_str(&appended);
                    next_node.detach();
                    child = Some(current);
                    continue;
                }
            }
            child = next;
        }
    }
}

pub fn remove_tag(document: &NodeRef, tag: &str) {
    let tag_nodes = document.select(tag).unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let as_node = tag_node.as_node();
        as_node.detach();
    }
}

/// Removes an element but keeps its children in its place -- Python's
/// lxml_html_clean.Cleaner(remove_tags=...), used for formatting tags
/// (b/strong/em/...) where the wrapper is noise but the text inside isn't.
pub fn unwrap_tag(document: &NodeRef, tag: &str) {
    let tag_nodes = document.select(tag).unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let as_node = tag_node.as_node();
        for child in as_node.children().collect::<Vec<_>>() {
            as_node.insert_before(child);
        }
        as_node.detach();
    }
}

pub fn trim_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    s.split_whitespace().for_each(|w| {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(w);
    });
    if result.is_empty() {
        return result;
    }
    trim_punctuation(result.as_str())
}

fn trim_punctuation(n: &str) -> String {
    let last_char = n.chars().last().unwrap();
    if last_char == '.' || last_char == ',' {
        let mut chars = n.chars();
        chars.next_back();
        return chars.as_str().to_string();
    }
    return n.to_string();
}

pub fn count_words(s: &str) -> usize {
    let mut total = 0;
    let mut previous = char::MAX;
    for c in s.chars() {
        // If previous char is whitespace, we are on a new word.
        if previous.is_ascii_whitespace() {
            // New word has alphabetic, digit or punctuation start.
            if c.is_ascii_alphabetic() || c.is_ascii_digit() || c.is_ascii_punctuation() {
                total += 1;
            }
        }
        // Set previous.
        previous = c;
    }
    if !s.is_empty() {
        total += 1
    }
    total
}

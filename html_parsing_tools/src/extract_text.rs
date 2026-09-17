use crate::text_nodes::form_text_nodes;
use crate::utils::{get_text_nodes, merge_adjacent_text_nodes, remove_tag, unwrap_tag};
use kuchiki::{traits::TendrilSink, NodeRef};
use std::collections::HashMap;

// Ported from oceanai/utils/html_nodes.py's content_cleaner config. Only the
// tags that affect visible text matter here -- content_cleaner also strips
// <meta>/inline style attrs/processing instructions, none of which itertext()
// would ever surface as text content, so there's nothing to replicate there.
const KILL_TAGS: [&str; 7] = ["script", "style", "noscript", "link", "frameset", "iframe", "area"];
const FORMATTING_TAGS: [&str; 15] = [
    "b", "big", "strong", "s", "i", "em", "mark", "small", "del", "ins", "sub", "sup", "strike", "tt", "u",
];

fn clean_content(document: &NodeRef) {
    for tag in KILL_TAGS {
        remove_tag(document, tag);
    }
    for tag in FORMATTING_TAGS {
        unwrap_tag(document, tag);
    }
    merge_adjacent_text_nodes(document);
}

fn is_cookie_disclaimer(node: &NodeRef) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    let attributes = element.attributes.borrow();
    let long_name = [
        attributes.get("class").unwrap_or(""),
        attributes.get("id").unwrap_or(""),
        attributes.get("name").unwrap_or(""),
    ]
    .join(" ")
    .to_lowercase();
    long_name.contains("cookie")
}

fn remove_cookie_disclaimers(document: &NodeRef) {
    let disclaimers: Vec<NodeRef> = document
        .inclusive_descendants()
        .filter(is_cookie_disclaimer)
        .collect();
    for node in disclaimers {
        node.detach();
    }
}

fn extract_text_elements(document: &NodeRef, tag: &str, min_split: Option<i32>) -> Option<Vec<Vec<String>>> {
    let elements: Vec<NodeRef> = document.select(tag).unwrap().map(|css| css.as_node().clone()).collect();
    let mut text_nodes: Vec<String> = vec![];
    for element in &elements {
        text_nodes.extend(get_text_nodes(element));
    }
    let result = form_text_nodes(&text_nodes, min_split);
    for element in elements {
        element.detach();
    }
    result
}

/// Rust port of oceanai/utils/html_nodes.py's `extract_text` +
/// `_extract_cleaned_body` + `_extract_text_elements`. Doesn't port
/// get_html_nodes/extract_meta/extract_title/get_display_selectors/
/// update_pages_text/the CSS display:none detection -- verified (grep, whole
/// repo) that none of them have any caller outside html_nodes.py itself, so
/// they're dead code in the Python source too, not just not-yet-ported.
pub fn extract_text(html: String, min_split: Option<i32>) -> HashMap<String, Vec<Vec<String>>> {
    let document = kuchiki::parse_html().one(html);
    clean_content(&document);

    let body = match document.select_first("body") {
        Ok(body) => body.as_node().clone(),
        // Python's fallback also strips any <title> left in the tree when
        // there's no <body> -- title text would otherwise leak into "main".
        Err(_) => {
            remove_tag(&document, "title");
            document.clone()
        }
    };

    remove_cookie_disclaimers(&body);

    let mut result = HashMap::new();

    if let Some(nav_text) = extract_text_elements(&body, "nav", min_split) {
        result.insert("navigation".to_string(), nav_text);
    }
    if let Some(footer_text) = extract_text_elements(&body, "footer", min_split) {
        result.insert("footer".to_string(), footer_text);
    }

    let main_text_nodes = get_text_nodes(&body);
    if let Some(main_text) = form_text_nodes(&main_text_nodes, min_split) {
        result.insert("main".to_string(), main_text);
    }

    result
}

/// Rust port of impressum.py's own inline use of remove_tree_elements: try
/// each text in order, and on the first one that matches at least one <a>
/// element whose text contains it, remove every matching anchor and stop
/// (don't also try the remaining texts) -- matches the `for ... if
/// elements_to_remove: remove; break` in Impressum.extract.
pub fn remove_matching_links(html: String, texts: Vec<String>) -> String {
    let document = kuchiki::parse_html().one(html);
    let Ok(anchors) = document.select("a") else {
        return document.to_string();
    };
    let anchors: Vec<NodeRef> = anchors.map(|css| css.as_node().clone()).collect();

    for text in texts {
        let matches: Vec<&NodeRef> = anchors
            .iter()
            .filter(|node| get_text_nodes(node).join("").contains(&text))
            .collect();
        if !matches.is_empty() {
            for node in matches {
                node.detach();
            }
            break;
        }
    }

    document.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from oceanai.utils.html_nodes.extract_text on the same HTML,
    // not hand-derived.
    const HTML: &str = "\
        <html><body>\
        <nav>Home | About | Contact</nav>\
        <script>var x = 1;</script>\
        <style>.foo{display:none}</style>\
        <div id=\"cookie-banner\" class=\"CookiesOK\">We use cookies</div>\
        <h1>Welcome to <b>Ocean</b>.io</h1>\
        <p>We help you find <strong>companies</strong> &amp;amp; people.</p>\
        <p>Second paragraph here.</p>\
        <footer>Copyright 2024 Ocean.io</footer>\
        </body></html>\
    ";

    #[test]
    fn test_extract_text_matches_python_reference() {
        let result = extract_text(HTML.to_string(), Some(0));

        assert_eq!(result["navigation"], [["Home | About | Contact"]]);
        assert_eq!(result["footer"], [["Copyright 2024 Ocean.io"]]);
        assert_eq!(
            result["main"],
            [
                ["Welcome to Ocean.io"],
                ["We help you find companies & people."],
                ["Second paragraph here."],
            ],
        );
        // The cookie banner's text must not survive anywhere in the output.
        for value in result.values() {
            for node in value {
                for sentence in node {
                    assert!(!sentence.contains("We use cookies"));
                }
            }
        }
    }

    #[test]
    fn test_remove_matching_links() {
        let html = "<html><body><a href=\"/x\">Read our Impressum here</a><p>keep</p></body></html>";
        let result = remove_matching_links(html.to_string(), vec!["Impressum".to_string(), "Imprint".to_string()]);
        assert!(!result.contains("Impressum"));
        assert!(result.contains("keep"));
    }

    #[test]
    fn test_remove_matching_links_stops_at_first_match() {
        // Both "Imprint" and "Legal notice" appear -- Python only removes
        // the first pattern's matches and stops, it doesn't also strip the
        // second.
        let html = "<html><body>\
            <a href=\"/a\">Imprint</a><a href=\"/b\">Legal notice</a>\
            </body></html>";
        let result = remove_matching_links(
            html.to_string(),
            vec!["Impressum".to_string(), "Imprint".to_string(), "Legal notice".to_string()],
        );
        assert!(!result.contains("Imprint"));
        assert!(result.contains("Legal notice"));
    }
}

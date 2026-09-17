mod extract_text;
mod regularize;
mod text_nodes;
mod utils;

use kuchiki::{iter::NodeIterator, traits::TendrilSink};
use linkify::{LinkFinder, LinkKind};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::{pyclass, pyfunction, pymodule, wrap_pyfunction, Bound, PyModule, PyModuleMethods, PyResult};
use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use std::collections::{HashMap, HashSet};
use text_nodes::*;
use utils::*;

const REMOVE_TAGS_HTML_CONTENTS: [&str; 3] = ["script", "style", "noscript"];

const REMOVE_TAGS: [&str; 27] = [
    // scripts/styles
    "script",
    "style",
    "noscript",
    // COOKIE BANNERS
    "#coiOverlay",
    ".CookiesOK",
    "#closeCookieBanner",
    ".CookieBanner-button",
    "#nts-set-cookie",
    ".cc_btn_accept_all",
    ".cookies",
    ".noticeCookiesContent .CustomDismissCtrl",
    ".cookie-consent .cookie-btn",
    "#accept-cookies",
    "#cookie_button_agree",
    "#cookies-agreement #agree-button",
    "#cookielayer .action-btn",
    ".cookie.nag .close",
    "#__tealiumGDPRecModal #consent_prompt_submit",
    ".gdpr__button",
    ".eu-cookie-compliance-agree-button",
    ".cookie-notification .js-cookie-notification-hide",
    ".js-accept-cookie-policy",
    "#moove_gdpr_cookie_info_bar",
    ".pea_cook_wrapper",
    // testimonials
    ".testimonial",
    ".testimonial-text",
    ".pwr-testimonial__quote", // hubspot
];

const PICK_TAGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

#[pyclass(get_all, module = "html_parsing_tools")]
#[derive(Default)]
struct GetSentencesResult {
    descriptions: Vec<String>,
    h: HashMap<String, Vec<String>>,
    json_ld: Vec<String>,
    keywords: String,
    other: Vec<String>,
    p: Vec<String>,
    text_nodes: Vec<Vec<String>>,
}

#[pyfunction]
#[pyo3(signature = (html, /, *, stop_word, remove_header, remove_footer, add_text_nodes, min_split_for_text_nodes))]
fn get_sentences(
    html: String,
    stop_word: &str,
    remove_header: bool,
    remove_footer: bool,
    add_text_nodes: bool,
    min_split_for_text_nodes: Option<i32>,
) -> PyResult<GetSentencesResult> {
    let mut result = GetSentencesResult::default();

    let document = kuchiki::parse_html().one(html);

    let json_ld = get_json_ld(&document);
    if !json_ld.is_empty() {
        result.json_ld = json_ld;
    }
    for tag in REMOVE_TAGS {
        remove_tag(&document, tag);
    }

    if remove_header {
        remove_tag(&document, "header");
        remove_tag(&document, "nav");
        remove_tag(&document, ".header");
        remove_tag(&document, ".header-hero");
    }

    if remove_footer {
        remove_tag(&document, "footer");
        remove_tag(&document, ".footer");
        remove_tag(&document, ".footer-hero");
    }

    let stop_word_regex = RegexBuilder::new(stop_word)
        .case_insensitive(true)
        .build()
        .expect("Invalid Regex");

    if add_text_nodes {
        let text_nodes: Vec<String> = document
            .inclusive_descendants()
            .text_nodes()
            .map(|text_node| text_node.borrow().to_string())
            .collect();
        let grouped_text_nodes = group_text_nodes(&text_nodes, min_split_for_text_nodes);
        if let Some(grouped_text_nodes) = grouped_text_nodes {
            result.text_nodes = grouped_text_nodes;
        }
    }

    for tag in PICK_TAGS {
        let text: Vec<String> = get_text_and_remove(&document, tag)
            .iter()
            .cloned()
            .collect();

        result
            .h
            .insert(tag.to_string(), apply(text, &stop_word_regex));
    }

    let mut paragraphs: Vec<String> = get_text_and_remove(&document, "p");
    paragraphs.sort_by(|a, b| count_words(b).cmp(&count_words(a)));

    let paragraphs: Vec<String> = paragraphs
        .iter()
        .filter(|x| count_words(x.as_str()) > 2)
        // .map(|x| x.split(". "))
        // .flatten()
        // .map(|x| x.split("! "))
        // .flatten()
        // .map(|x| x.split("? "))
        // .flatten()
        .map(|x| x.to_string())
        // .filter(|x| count_words(x.as_str()) < 128)
        // .take(30)
        .collect();

    result.p = apply(paragraphs, &stop_word_regex);

    let descriptions = get_descriptions(&document);
    if !descriptions.is_empty() {
        result.descriptions = descriptions;
    }

    let keywords = get_keywords(&document);
    if let Some(keywords) = keywords {
        result.keywords = keywords;
    }

    result.other = get_text_nodes(&document);

    Ok(result)
}

#[pyfunction]
#[pyo3(signature = (htmls, /, *, stop_word, remove_header, remove_footer, add_text_nodes, min_split_for_text_nodes))]
fn get_sentences_parallel(
    htmls: Vec<String>,
    stop_word: &str,
    remove_header: bool,
    remove_footer: bool,
    add_text_nodes: bool,
    min_split_for_text_nodes: Option<i32>,
) -> PyResult<Vec<GetSentencesResult>> {
    htmls
        .into_par_iter()
        .map(|html| {
            get_sentences(
                html,
                stop_word,
                remove_header,
                remove_footer,
                add_text_nodes,
                min_split_for_text_nodes,
            )
        })
        .collect()
}

#[pyfunction]
fn get_href_attributes(html: String) -> PyResult<Vec<String>> {
    let document = kuchiki::parse_html().one(html);
    // let mut links: Vec<String> = vec![];

    let links: Vec<String> = document
        .select("a")
        .unwrap()
        // .collect()
        .map(|x| {
            let attributes = x.attributes.borrow();
            let href = attributes.get("href");
            if href.is_none() {
                return "".to_string();
            }
            href.unwrap().to_string()
        })
        .collect();

    Ok(links)
}

#[pyfunction]
fn get_links(html: String) -> PyResult<Vec<(String, String)>> {
    let document = kuchiki::parse_html().one(html);
    // let mut links: Vec<String> = vec![];

    let links: Vec<(String, String)> = document
        .select("a")
        .unwrap()
        // .collect()
        .map(|x| {
            let attributes = x.attributes.borrow();
            let href = attributes.get("href");
            let text = get_text_string(x.as_node(), " ");
            if href.is_none() {
                return ("".to_string(), text);
            }
            return (href.unwrap().to_string(), text);
        })
        .collect();

    Ok(links)
}

#[pyfunction]
fn get_emails(html: String) -> PyResult<Vec<String>> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Email]);
    let links = finder
        .links(html.as_str())
        .map(|x| x.as_str().trim_matches('\'').to_string())
        .collect::<Vec<String>>();
    Ok(links)
}

#[pyfunction]
fn get_meta_titles(html: String) -> PyResult<HashMap<String, String>> {
    let document = kuchiki::parse_html().one(html);
    let mut result: HashMap<String, String> = HashMap::new();
    let tag_nodes = document.select("meta").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes: std::cell::Ref<kuchiki::Attributes> = tag_node.attributes.borrow();
        let name_attribute = attributes.get("name").unwrap_or("");
        if name_attribute == "twitter:title" || name_attribute == "og:title" {
            let content = attributes.get("content").unwrap_or("").to_string();
            if content.is_empty() {
                continue;
            }
            result.insert(name_attribute.to_string(), content);
        }
    }
    let tag_nodes: kuchiki::iter::Select<kuchiki::iter::Elements<kuchiki::iter::Descendants>> =
        document.select("title").unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        result.insert(
            "title".to_string(),
            get_text_string(tag_node.as_node(), " "),
        );
    }

    Ok(result)
}

#[pyfunction]
fn tag_attribute(html: String, tag: String, attribute: String) -> PyResult<String> {
    let document = kuchiki::parse_html().one(html);
    let tag_nodes: kuchiki::iter::Select<kuchiki::iter::Elements<kuchiki::iter::Descendants>> =
        document.select(tag.as_str()).unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes: std::cell::Ref<kuchiki::Attributes> = tag_node.attributes.borrow();
        return Ok(attributes.get(attribute).unwrap_or("").to_string());
    }

    Ok("".to_string())
}

#[pyfunction]
fn get_alternate_links(html: String) -> PyResult<HashMap<String, Vec<String>>> {
    let document = kuchiki::parse_html().one(html);
    Ok(get_rel_alternate(&document))
}

#[pyfunction]
fn html_contents(html: String) -> PyResult<String> {
    let document = kuchiki::parse_html().one(html);
    for tag in REMOVE_TAGS_HTML_CONTENTS {
        remove_tag(&document, tag);
    }
    Ok(document.to_string())
}

#[pyfunction]
fn tag_html_contents(html: String, tag: String) -> PyResult<String> {
    let document = kuchiki::parse_html().one(html);
    let document = document.select_first(tag.as_str());
    let res = match document {
        Ok(v) => v.as_node().to_string(),
        Err(_) => "".to_string(),
    };

    Ok(res)
}

#[pyfunction]
fn get_lang(html: String) -> PyResult<String> {
    let document = kuchiki::parse_html().one(html);
    Ok(get_lang_internal(&document))
}

/// Markdown for one page, headings and paragraphs in document order.
///
/// get_sentences buckets by tag (every h1, then every h2, ...) and sorts
/// paragraphs by word count -- good for keyword/embedding pipelines, useless
/// for reconstructing a readable page. This walks h1-h6 and p together in a
/// single selector query, which kuchiki/selectors resolves as one
/// document-order traversal, so headings and paragraphs interleave the way
/// they actually appear on the page (levels are still flattened relative to
/// each other -- an h3 nested under an h2 nested under an h1 all just come
/// out as their own "###"/"##"/"#" in sequence, since nothing here tracks
/// heading nesting, only encounter order).
#[pyfunction]
#[pyo3(signature = (html, /, *, stop_word, remove_header, remove_footer))]
fn get_markdown(
    html: String,
    stop_word: &str,
    remove_header: bool,
    remove_footer: bool,
) -> PyResult<String> {
    let document = kuchiki::parse_html().one(html);

    for tag in REMOVE_TAGS {
        remove_tag(&document, tag);
    }

    if remove_header {
        remove_tag(&document, "header");
        remove_tag(&document, "nav");
        remove_tag(&document, ".header");
        remove_tag(&document, ".header-hero");
    }

    if remove_footer {
        remove_tag(&document, "footer");
        remove_tag(&document, ".footer");
        remove_tag(&document, ".footer-hero");
    }

    let stop_word_regex = RegexBuilder::new(stop_word)
        .case_insensitive(true)
        .build()
        .expect("Invalid Regex");

    let mut blocks: Vec<String> = Vec::new();
    let matches = document
        .select("h1, h2, h3, h4, h5, h6, p")
        .expect("Invalid selector");

    for tag_node in matches {
        let tag_name = tag_node.name.local.to_string();
        // Same trim get_text_and_remove applies (trailing '.'/',' included) --
        // get_sentences' own h1-h6/p output goes through the same step.
        let raw_text = trim_whitespace(get_text_string(tag_node.as_node(), " ").as_str());

        // Same order get_sentences applies it in: the word-count floor is a
        // noise filter on the *original* text (mirrors its `paragraphs.iter()
        // .filter(count_words(x) > 2)` before that same text goes through
        // `apply` -- a stop_word that hollows a paragraph out to 1-2 words
        // shouldn't save it from the floor it would've failed anyway).
        if tag_name == "p" && count_words(&raw_text) <= 2 {
            continue;
        }

        // Reuses get_sentences' own cleaning (stop-word strip, cookie-banner
        // filtering) so a heading/paragraph that would've been dropped there
        // is dropped here too, same rules either way.
        let Some(text) = apply(vec![raw_text], &stop_word_regex).pop() else {
            continue;
        };

        if tag_name == "p" {
            blocks.push(text);
        } else {
            // PICK_TAGS is exactly "h1".. "h6", so this is always 1-6.
            let level: usize = tag_name[1..].parse().unwrap_or(1);
            blocks.push(format!("{} {}", "#".repeat(level), text));
        }
    }

    Ok(blocks.join("\n\n"))
}

#[pyfunction]
#[pyo3(signature = (htmls, /, *, stop_word, remove_header, remove_footer))]
fn get_markdown_parallel(
    htmls: Vec<String>,
    stop_word: &str,
    remove_header: bool,
    remove_footer: bool,
) -> PyResult<Vec<String>> {
    htmls
        .into_par_iter()
        .map(|html| get_markdown(html, stop_word, remove_header, remove_footer))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = "\
        <html>
        <head>
        <meta property=\"og:title\" content=\"meta title\">
        <meta name=\"description\" content=\"meta description\">
        <meta name=\"description\" content=\"meta description\">
        <meta property=\"og:description\" content=\"meta og:description\">
        <meta property=\"og:description\">
        <meta property=\"og:description\" content=\"\">
        </head>
        <body>
        <h1>H1            header &pound;100 &#42030550023695;<br>the same H1 on a new line</h1>
        <p>p _stop_ tag</p><p>another p on the same line</p>
        <div><p>p, next should be span without leading space:</p><span>span1</span></div><div class=\"class1\"><span>span2 should be on its own</span></div>
        <div><span>span3 should be on its own</span></div>
        <p>home a/s<br>Frichsparken<br>Søren Frichs Vej 36 F<br>8230 Åbyhøj<br>CVR: 13394172<br>Telefon: 86 15 43 00<br>Email: <a href=\"mailto:homeas@home.dk\">homeas@home.dk</a></p>
        </body>
        </html>
    ";

    #[test]
    fn test_get_sentences() {
        let result = get_sentences(HTML.to_string(), "_stop_", false, false, true, None).unwrap();

        assert_eq!(
            result.text_nodes,
            [
                vec!["H1 header £100 �", "the same H1 on a new line"],
                vec!["p _stop_ tag", "another p on the same line"],
                vec![
                    "p, next should be span without leading space: span1",
                    "span2 should be on its own"
                ],
                vec!["span3 should be on its own"],
                vec![
                    "home a/s",
                    "Frichsparken",
                    "Søren Frichs Vej 36 F",
                    "8230 Åbyhøj",
                    "CVR: 13394172",
                    "Telefon: 86 15 43 00",
                    "Email: homeas@home.dk"
                ],
            ],
        );
        assert_eq!(
            result.h["h1"],
            ["H1 header £100 � the same H1 on a new line"]
        );
        assert_eq!(
            result.p,
            [
                "home a/s Frichsparken Søren Frichs Vej 36 F 8230 Åbyhøj CVR: 13394172 Telefon: 86 15 43 00 Email: homeas@home.dk",
                "p, next should be span without leading space:",
                "another p on the same line",
                "p tag",
            ],
        );
        assert_eq!(
            result.other,
            [
                "span1",
                "span2 should be on its own",
                "span3 should be on its own",
            ]
        );
        assert_eq!(
            result.descriptions,
            ["meta description", "meta og:description"]
        );

        let result = get_sentences(
            "<html><head></head></html>".to_string(),
            "_stop_",
            false,
            false,
            true,
            None,
        )
        .unwrap();
        assert!(result.text_nodes.is_empty());
    }

    #[test]
    fn test_get_markdown() {
        let result = get_markdown(HTML.to_string(), "_stop_", false, false).unwrap();

        assert_eq!(
            result,
            "# H1 header \u{a3}100 \u{fffd} the same H1 on a new line\n\n\
             p tag\n\n\
             another p on the same line\n\n\
             p, next should be span without leading space:\n\n\
             home a/s Frichsparken S\u{f8}ren Frichs Vej 36 F 8230 \u{c5}byh\u{f8}j \
             CVR: 13394172 Telefon: 86 15 43 00 Email: homeas@home.dk"
        );

        // Headings and paragraphs interleave in document order -- the h1
        // comes before every p, not bucketed separately the way
        // get_sentences' own h1/p fields are.
        let heading_pos = result.find("# H1").unwrap();
        let first_p_pos = result.find("p tag").unwrap();
        assert!(heading_pos < first_p_pos);

        let empty = get_markdown(
            "<html><head></head></html>".to_string(),
            "_stop_",
            false,
            false,
        )
        .unwrap();
        assert_eq!(empty, "");
    }

    #[test]
    fn test_get_markdown_heading_levels_and_short_paragraphs_are_dropped() {
        let html = "\
            <h1>Ocean.io</h1>\
            <p>Search for companies in plain language and get matched accounts instantly.</p>\
            <h2>How it works</h2>\
            <p>Type the domain you want to find the look-alike to and our AI does the rest.</p>\
            <h3>Too short</h3>\
            <p>Two words</p>\
        "
        .to_string();

        let result = get_markdown(html, "_stop_", false, false).unwrap();

        assert_eq!(
            result,
            "# Ocean.io\n\n\
             Search for companies in plain language and get matched accounts instantly\n\n\
             ## How it works\n\n\
             Type the domain you want to find the look-alike to and our AI does the rest\n\n\
             ### Too short"
        );
        // "Two words" is exactly 2 words -- below the paragraph floor, so it's
        // the heading that survives here, not the paragraph under it.
        assert!(!result.contains("Two words"));
    }

    #[test]
    fn test_get_markdown_parallel() {
        let result = get_markdown_parallel(
            vec![HTML.to_string(), HTML.to_string()],
            "_stop_",
            false,
            false,
        )
        .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], result[1]);
        assert!(result[0].starts_with("# H1"));
    }

    #[test]
    fn test_get_sentences_parallel() {
        let result = get_sentences_parallel(
            vec![HTML.to_string(), HTML.to_string()],
            "_stop_",
            false,
            false,
            true,
            None,
        )
        .unwrap();
        assert_eq!(result.len(), 2);

        let text_nodes = [
            vec!["H1 header £100 �", "the same H1 on a new line"],
            vec!["p _stop_ tag", "another p on the same line"],
            vec![
                "p, next should be span without leading space: span1",
                "span2 should be on its own",
            ],
            vec!["span3 should be on its own"],
            vec![
                "home a/s",
                "Frichsparken",
                "Søren Frichs Vej 36 F",
                "8230 Åbyhøj",
                "CVR: 13394172",
                "Telefon: 86 15 43 00",
                "Email: homeas@home.dk",
            ],
        ];

        assert_eq!(result[0].text_nodes, text_nodes);
        assert_eq!(result[1].text_nodes, text_nodes);
    }

    #[test]
    fn test_get_emails() {
        let html = "\
            <p>You can always reach out to Soren, Anders and Teffi who are responsible for the web-shop via \
            <a href=\"mailto:shop@respectresources.dk\" \
            onclick=\"return rcmail.command('compose','shop@respectresources.dk',this)\">shop@respectresources.dk</a>.</p>
            ".to_string();

        let result = get_emails(html).unwrap();
        assert_eq!(
            result,
            [
                "shop@respectresources.dk",
                "shop@respectresources.dk",
                "shop@respectresources.dk",
            ]
        );
    }
}

#[pyfunction(name = "regularize")]
fn regularize_py(text: String) -> String {
    regularize::regularize(&text)
}

/// split_re defaults to the same SENTENCE_SPLIT_RE get_sentences/get_markdown
/// use internally; pass "title" for company_name.py's TITLE_SPLIT_RE use, or
/// any other pattern string for a custom split (Rust's regex crate supports
/// the same \p{...} Unicode-category syntax the Python `regex` module does).
#[pyfunction(name = "split_sentence")]
#[pyo3(signature = (sentence, /, *, split_re=None, split_words=None))]
fn split_sentence_py(sentence: String, split_re: Option<String>, split_words: Option<HashSet<String>>) -> PyResult<Vec<String>> {
    let compiled;
    let re: &Regex = match split_re.as_deref() {
        None => &text_nodes::SENTENCE_SPLIT_RE,
        Some("title") => &text_nodes::TITLE_SPLIT_RE,
        Some(pattern) => {
            compiled = RegexBuilder::new(pattern)
                .build()
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            &compiled
        }
    };
    let words_set: Option<HashSet<&str>> = split_words.as_ref().map(|v| v.iter().map(String::as_str).collect());
    Ok(text_nodes::split_sentence(&sentence, re, words_set.as_ref()))
}

/// Rust port of oceanai/utils/text_nodes.py's form_text_nodes -- text_nodes,
/// grouped and regularized, from a flat sequence of extracted text elements
/// (e.g. lxml's `element.itertext()`, or this crate's own
/// get_text_nodes-shaped output).
#[pyfunction(name = "form_text_nodes")]
#[pyo3(signature = (text_elements, min_split=None))]
fn form_text_nodes_py(text_elements: Vec<String>, min_split: Option<i32>) -> Vec<Vec<String>> {
    text_nodes::form_text_nodes(&text_elements, min_split).unwrap_or_default()
}

/// Rust port of oceanai/utils/html_nodes.py's extract_text (+ its private
/// _extract_cleaned_body/_extract_text_elements helpers). Does not cover
/// extract_meta/extract_title/get_html_nodes/get_display_selectors/
/// update_pages_text -- verified those have no caller anywhere outside
/// html_nodes.py itself, so they're dead code in the Python source, not
/// unported functionality.
#[pyfunction(name = "extract_text")]
#[pyo3(signature = (html, /, *, min_split=None))]
fn extract_text_py(html: String, min_split: Option<i32>) -> HashMap<String, Vec<Vec<String>>> {
    extract_text::extract_text(html, min_split)
}

#[pyfunction]
fn remove_matching_links(html: String, texts: Vec<String>) -> String {
    extract_text::remove_matching_links(html, texts)
}

/// A Python module implemented in Rust.
#[pymodule]
fn html_parsing_tools(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_emails, m)?)?;
    m.add_function(wrap_pyfunction!(get_links, m)?)?;
    m.add_function(wrap_pyfunction!(html_contents, m)?)?;
    m.add_function(wrap_pyfunction!(tag_html_contents, m)?)?;
    m.add_function(wrap_pyfunction!(tag_attribute, m)?)?;
    m.add_function(wrap_pyfunction!(get_sentences, m)?)?;
    m.add_function(wrap_pyfunction!(get_sentences_parallel, m)?)?;
    m.add_function(wrap_pyfunction!(get_markdown, m)?)?;
    m.add_function(wrap_pyfunction!(get_markdown_parallel, m)?)?;
    m.add_function(wrap_pyfunction!(get_href_attributes, m)?)?;
    m.add_function(wrap_pyfunction!(get_alternate_links, m)?)?;
    m.add_function(wrap_pyfunction!(get_lang, m)?)?;
    m.add_function(wrap_pyfunction!(get_meta_titles, m)?)?;
    m.add_function(wrap_pyfunction!(regularize_py, m)?)?;
    m.add_function(wrap_pyfunction!(split_sentence_py, m)?)?;
    m.add_function(wrap_pyfunction!(form_text_nodes_py, m)?)?;
    m.add_function(wrap_pyfunction!(extract_text_py, m)?)?;
    m.add_function(wrap_pyfunction!(remove_matching_links, m)?)?;
    m.add_class::<GetSentencesResult>()?;
    Ok(())
}

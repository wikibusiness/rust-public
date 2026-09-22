mod extract_text;
#[cfg(test)]
mod golden_tests;
mod regularize;
mod text_nodes;
mod utils;

use kuchikiki::{iter::NodeIterator, traits::TendrilSink};
use linkify::{LinkFinder, LinkKind};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::{pyclass, pyfunction, pymethods, pymodule, wrap_pyfunction, Bound, PyModule, PyModuleMethods, PyResult};
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
    // Same parse as everything else above -- a caller that already has a
    // GetSentencesResult for a page should read these instead of calling
    // get_meta_titles/get_href_attributes and reparsing the page's HTML.
    meta_titles: HashMap<String, String>,
    href_attributes: Vec<String>,
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

    let document = kuchikiki::parse_html().one(html);

    // Captured before any tag is removed below, so these match exactly what
    // the standalone get_meta_titles/get_href_attributes calls would return
    // for the same (untouched) HTML.
    result.meta_titles = get_meta_titles_internal(&document);
    result.href_attributes = get_href_attributes_internal(&document);

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
    let document = kuchikiki::parse_html().one(html);
    Ok(get_href_attributes_internal(&document))
}

#[pyfunction]
fn get_links(html: String) -> PyResult<Vec<(String, String)>> {
    let document = kuchikiki::parse_html().one(html);
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
    let document = kuchikiki::parse_html().one(html);
    Ok(get_meta_titles_internal(&document))
}

#[pyfunction]
fn tag_attribute(html: String, tag: String, attribute: String) -> PyResult<String> {
    let document = kuchikiki::parse_html().one(html);
    let tag_nodes: kuchikiki::iter::Select<kuchikiki::iter::Elements<kuchikiki::iter::Descendants>> =
        document.select(tag.as_str()).unwrap();
    for tag_node in tag_nodes.collect::<Vec<_>>() {
        let attributes: std::cell::Ref<kuchikiki::Attributes> = tag_node.attributes.borrow();
        return Ok(attributes.get(attribute).unwrap_or("").to_string());
    }

    Ok("".to_string())
}

#[pyfunction]
fn get_alternate_links(html: String) -> PyResult<HashMap<String, Vec<String>>> {
    let document = kuchikiki::parse_html().one(html);
    Ok(get_rel_alternate(&document))
}

#[pyfunction]
fn html_contents(html: String) -> PyResult<String> {
    let document = kuchikiki::parse_html().one(html);
    for tag in REMOVE_TAGS_HTML_CONTENTS {
        remove_tag(&document, tag);
    }
    Ok(document.to_string())
}

#[pyfunction]
fn tag_html_contents(html: String, tag: String) -> PyResult<String> {
    let document = kuchikiki::parse_html().one(html);
    let document = document.select_first(tag.as_str());
    let res = match document {
        Ok(v) => v.as_node().to_string(),
        Err(_) => "".to_string(),
    };

    Ok(res)
}

#[pyfunction]
fn get_lang(html: String) -> PyResult<String> {
    let document = kuchikiki::parse_html().one(html);
    Ok(get_lang_internal(&document))
}

/// Markdown for one page, headings and paragraphs in document order.
///
/// get_sentences buckets by tag (every h1, then every h2, ...) and sorts
/// paragraphs by word count -- good for keyword/embedding pipelines, useless
/// for reconstructing a readable page. This walks h1-h6 and p together in a
/// single selector query, which kuchikiki/selectors resolves as one
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
    let document = kuchikiki::parse_html().one(html);

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
        // get_meta_titles only recognizes <meta name="og:title"/"twitter:title">,
        // not property= (that's get_descriptions' job, and it does check both) --
        // HTML's own og:title tag above uses property=, so it's correctly absent here.
        assert!(result.meta_titles.is_empty());
        assert_eq!(result.href_attributes, ["mailto:homeas@home.dk"]);

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
    fn test_get_sentences_meta_titles_and_href_attributes_match_standalone_calls() {
        // The whole point of carrying these on GetSentencesResult is that a
        // caller who already parsed a page via get_sentences/get_sentences_parallel
        // gets the same answer get_meta_titles/get_href_attributes would have
        // given on a fresh parse of the same HTML, without reparsing it.
        let html = HTML.to_string();

        let sentences =
            get_sentences(html.clone(), "_stop_", false, false, true, None).unwrap();
        let standalone_meta_titles = get_meta_titles(html.clone()).unwrap();
        let standalone_href_attributes = get_href_attributes(html).unwrap();

        assert_eq!(sentences.meta_titles, standalone_meta_titles);
        assert_eq!(sentences.href_attributes, standalone_href_attributes);
        // Non-trivial, so this isn't just two empty collections trivially
        // matching each other (HTML's meta tags use property=, which
        // get_meta_titles doesn't match -- see test_get_sentences).
        assert!(!sentences.href_attributes.is_empty());

        // A page whose meta tags actually use name= (the form get_meta_titles
        // does match) exercises the non-empty case for that field too.
        let with_name_meta = "<html><head><meta name=\"og:title\" content=\"Meta Title\">\
            <title>Page Title</title></head><body><a href=\"/a\">x</a></body></html>"
            .to_string();
        let sentences = get_sentences(
            with_name_meta.clone(),
            "_stop_",
            false,
            false,
            true,
            None,
        )
        .unwrap();
        let standalone_meta_titles = get_meta_titles(with_name_meta).unwrap();
        assert_eq!(sentences.meta_titles, standalone_meta_titles);
        assert!(!sentences.meta_titles.is_empty());
    }

    #[test]
    fn test_parsed_page_get_anchor_links() {
        let html = "<a href=\"/plain\">Investor Relations</a>\
            <a href=\"/nested\"><b>Investor</b> Relations</a>\
            <a>no href</a>"
            .to_string();
        let page = load_page(html);
        assert_eq!(
            page.get_anchor_links(),
            [
                ("/plain".to_string(), "Investor Relations".to_string()),
                // lxml's `.text` on <a><b>Investor</b> Relations</a> is None:
                // the first child is the <b> element, not a text node, so
                // the text lives in the *tail* of <b>, which .text doesn't
                // see. Reproduced here as an empty string, not fixed.
                ("/nested".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_parsed_page_get_link_attributes() {
        let html = "<img src=\"/logo.png\"><a href=\"/a\">x</a>\
            <div data-href=\"/custom\">y</div>"
            .to_string();
        let page = load_page(html);
        let mut links = page.get_link_attributes();
        links.sort();
        assert_eq!(links, ["/a", "/custom", "/logo.png"]);
    }

    #[test]
    fn test_parsed_page_get_link_elements() {
        let html = "<img src=\"/logo.png\" width=\"32\" alt=\"Logo\">".to_string();
        let page = load_page(html);
        let elements = page.get_link_elements();
        assert_eq!(elements.len(), 1);
        let (tag, href, attrs) = &elements[0];
        assert_eq!(tag, "img");
        assert_eq!(href, "/logo.png");
        assert_eq!(attrs.get("width").map(String::as_str), Some("32"));
        assert_eq!(attrs.get("alt").map(String::as_str), Some("Logo"));
    }

    #[test]
    fn test_parsed_page_get_script_contents() {
        let html = "<script type=\"text/javascript\">var x = 1 < 2;</script>".to_string();
        let page = load_page(html);
        let scripts = page.get_script_contents();
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].contains("var x = 1"));
        assert!(scripts[0].starts_with("<script"));
    }

    #[test]
    fn test_parsed_page_get_meta_tags() {
        let html = "<meta name=\"description\" content=\"d\">\
            <meta property=\"og:title\" content=\"t\">\
            <meta content=\"skipped, no name or property\">"
            .to_string();
        let page = load_page(html);
        assert_eq!(
            page.get_meta_tags(),
            [
                ("description".to_string(), "d".to_string()),
                ("og:title".to_string(), "t".to_string()),
            ]
        );
    }

    #[test]
    fn test_parsed_page_get_anchor_text_fragments() {
        let html = "<a href=\"/a\">Hello <b>World</b></a><a>no href</a>".to_string();
        let page = load_page(html);
        assert_eq!(
            page.get_anchor_text_fragments(),
            [
                (Some("/a".to_string()), vec!["Hello ".to_string(), "World".to_string()]),
                (None, vec!["no href".to_string()]),
            ]
        );
    }

    #[test]
    fn test_parsed_page_get_script_texts() {
        let html = "<script src=\"/a.js\">var x = 1;</script><script>no src</script>".to_string();
        let page = load_page(html);
        assert_eq!(
            page.get_script_texts(),
            [
                ("var x = 1;".to_string(), "/a.js".to_string()),
                ("no src".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_parsed_page_get_elements() {
        let html = "<div class=\"cart-icon\">x</div><video></video>".to_string();
        let page = load_page(html);
        let elements = page.get_elements();
        let div = elements.iter().find(|(tag, _)| tag == "div").unwrap();
        assert_eq!(div.1.get("class").map(String::as_str), Some("cart-icon"));
        assert!(elements.iter().any(|(tag, _)| tag == "video"));
    }

    #[test]
    fn test_parsed_page_get_json_ld() {
        let html = "<script type=\"application/ld+json\">{\"@type\":\"Organization\"}</script>\
            <script>not json_ld</script>"
            .to_string();
        let page = load_page(html);
        assert_eq!(page.get_json_ld(), ["{\"@type\":\"Organization\"}"]);
    }

    #[test]
    fn test_strip_buttons_and_get_text() {
        // Both captured from real lxml output (fromstring -> remove(button)
        // via getparent() -> tostring(method="text")), not hand-derived --
        // the tail-text-goes-with-the-button behavior is easy to get wrong.
        assert_eq!(
            strip_buttons_and_get_text(
                "<p class=\"x\">Keep <button>Click me</button> this text</p>".to_string()
            ),
            "Keep "
        );
        assert_eq!(
            strip_buttons_and_get_text(
                "<p><button>A</button>text1<button>B</button>text2</p>".to_string()
            ),
            ""
        );
        assert_eq!(
            strip_buttons_and_get_text(
                "<div>a<button>b</button><span>c</span>d</div>".to_string()
            ),
            "acd"
        );
    }

    #[test]
    fn test_strip_buttons_and_get_text_no_buttons() {
        let html = "<p>Just text</p>".to_string();
        assert_eq!(strip_buttons_and_get_text(html), "Just text");
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

/// Drop every `<button>` element and return the concatenation of all
/// remaining text nodes, no separator inserted -- matches
/// `parent.remove(button); tostring(node, method="text")` on lxml's tree,
/// including the part that's easy to miss: ElementTree's `.remove()` drops
/// the removed element's *tail* text too (the text node immediately after
/// it, up to the next sibling), not just the element itself. Verified
/// against real lxml output, not assumed -- `<p>Keep <button>x</button> this
/// text</p>` loses " this text" along with the button on both sides.
/// Replaces the fromstring -> find buttons -> remove -> tostring(text)
/// pipeline common/utils/job_description.py used lxml for; unlike lxml,
/// this never raises on malformed input, so there's no parse-failure path
/// to fall back from.
#[pyfunction]
fn strip_buttons_and_get_text(html: String) -> String {
    let document = kuchikiki::parse_html().one(html);
    let buttons: Vec<kuchikiki::NodeRef> = document
        .select("button")
        .unwrap()
        .map(|css| css.as_node().clone())
        .collect();
    for button in buttons {
        if let Some(next) = button.next_sibling() {
            if next.as_text().is_some() {
                next.detach();
            }
        }
        button.detach();
    }
    document.text_contents()
}

// The standard HTML link-bearing attributes lxml.html.defs.link_attrs
// enumerates (verified against the real frozenset, not guessed).
const LINK_ATTRS: [&str; 15] = [
    "longdesc", "cite", "src", "classid", "usemap", "formaction", "href",
    "action", "lowsrc", "profile", "codebase", "background", "data",
    "archive", "dynsrc",
];

/// A page parsed once so callers can pull out different sub-parts (links,
/// scripts, meta tags, ...) without reparsing -- the replacement for handing
/// a caller an lxml tree, without lxml's per-node Python object memory
/// overhead.
///
/// Every sub-part is computed eagerly in `load_page`, not lazily per method
/// call: kuchikiki's tree is `Rc`-based (not `Send`/`Sync`), and this crate's
/// only real caller (ai_center's html_content_processing pipeline) fans
/// extractors out across `asyncio.to_thread` -- a genuinely different OS
/// thread per extractor, not just concurrent-under-the-GIL. A pyclass
/// holding a live `Rc` tree would need `unsendable`, which panics the moment
/// a different thread touches it, i.e. on the very next extractor call.
/// Computing everything up front into plain owned data sidesteps that: the
/// class is trivially `Send`/`Sync` and safe to share across the fan-out.
#[pyclass(module = "html_parsing_tools")]
struct ParsedPage {
    anchor_links: Vec<(String, String)>,
    anchor_text_fragments: Vec<(Option<String>, Vec<String>)>,
    link_attributes: Vec<String>,
    link_elements: Vec<(String, String, HashMap<String, String>)>,
    script_contents: Vec<String>,
    script_texts: Vec<(String, String)>,
    meta_tags: Vec<(String, String)>,
    json_ld: Vec<String>,
    elements: Vec<(String, HashMap<String, String>)>,
}

#[pymethods]
impl ParsedPage {
    /// (href, text) for every `<a href>`. `text` matches lxml's `element.text`
    /// -- only the text node immediately after the opening tag, not text
    /// inside a nested child element (`<a><b>x</b></a>` yields text="", same
    /// as lxml's `.text` being None there) and not the tail. This is the
    /// exact (slightly-misses-nested-markup) behavior company_data.py's
    /// contains_investor_info already relied on via lxml -- reproduced as-is,
    /// not fixed.
    fn get_anchor_links(&self) -> Vec<(String, String)> {
        self.anchor_links.clone()
    }

    /// Every attribute value lxml's `iterlinks()` would surface as a link:
    /// the standard link-bearing attributes (href/src/action/data/...) on any
    /// tag, plus literal `data-href` attributes (a site-specific convention
    /// this codebase also scans for via `//@data-href`).
    ///
    /// ponytail: doesn't replicate iterlinks()'s <object>/<param>
    /// codebase-relative joining, <meta http-equiv="refresh"> redirect
    /// targets, or CSS `url()`/`@import` extraction from <style> tags/style
    /// attributes -- all rare document shapes, and the only caller
    /// (get_pages_absolute_links) filters down to absolute http(s) links
    /// anyway, which most of those forms aren't. Upgrade path: port the
    /// remaining branches of lxml.html.HtmlMixin.iterlinks here if a real
    /// domain is found needing them.
    fn get_link_attributes(&self) -> Vec<String> {
        self.link_attributes.clone()
    }

    /// Same scan as get_link_attributes, but carrying the tag name and full
    /// attribute map of the element each link came from -- for callers (like
    /// logo.py's image-link scoring) that need to read other attributes
    /// (width/height/alt/class/...) off that same element, which a flat href
    /// list can't give them. `attribute`/lxml's per-link char position from
    /// iterlinks() aren't carried: logo.py's own iterlinks() consumption
    /// never used them either (verified at the call site).
    fn get_link_elements(&self) -> Vec<(String, String, HashMap<String, String>)> {
        self.link_elements.clone()
    }

    /// Outer HTML of every `<script>` element (tag, attributes, and raw
    /// content), matching lxml's `tostring(script_element)`.
    fn get_script_contents(&self) -> Vec<String> {
        self.script_contents.clone()
    }

    /// (key, content) per `<meta>` tag -- key is the `name` attribute if
    /// present, else `property`; tags with neither are skipped. Matches
    /// _tech_detector.py's _extract_meta_tags.
    fn get_meta_tags(&self) -> Vec<(String, String)> {
        self.meta_tags.clone()
    }

    /// Raw text content of every `<script type="application/ld+json">` tag,
    /// same extraction get_sentences already uses for its own json_ld field
    /// (see get_json_ld in utils.rs) -- exposed here for callers (like
    /// oceanai.utils.web's get_json_linked_data) that need it without also
    /// running full sentence extraction.
    fn get_json_ld(&self) -> Vec<String> {
        self.json_ld.clone()
    }

    /// (href, text_fragments) for every `<a>` tag, href absent (None) if the
    /// attribute itself is missing (distinct from present-but-empty, which
    /// is `Some("")`). `text_fragments` is every descendant text node's raw,
    /// untrimmed content in document order -- the equivalent of lxml's
    /// `element.itertext()`, meant to be fed to this crate's own
    /// `form_text_nodes` exactly like the original Python code did with
    /// itertext() output (triggers.py/gtm.py's `_get_element_text`/
    /// `_element_text`). Deliberately not reusing get_anchor_links' shallow
    /// `.text`-only field here -- that's a different (and, for this caller,
    /// wrong) text scope.
    fn get_anchor_text_fragments(&self) -> Vec<(Option<String>, Vec<String>)> {
        self.anchor_text_fragments.clone()
    }

    /// (text, src) per `<script>` tag: text is the tag's direct text content
    /// (script/style are HTML5 "raw text" elements with a single text child,
    /// so this is the whole content, matching lxml's `element.text`), src is
    /// the `src` attribute or "" if absent. For callers that want the script
    /// body itself, not the serialized tag (see get_script_contents for that).
    fn get_script_texts(&self) -> Vec<(String, String)> {
        self.script_texts.clone()
    }

    /// (tag_name, attributes) for every element in the document, document
    /// order. A deliberately generic dump rather than another bespoke
    /// query: several callers (triggers.py's config-driven xpath patterns,
    /// gtm.py's embed-signal xpath patterns, logo.py's apple-touch-icon
    /// lookup and its per-link width/height/alt/class reads) each need a
    /// different, small slice of "tag + attributes", and the matching logic
    /// itself lives in Python next to the config data it reads (TRIGGERS,
    /// GTM_SIGNALS) -- there's no reason to also duplicate an xpath-pattern
    /// interpreter in Rust for it. Cheap: pages are already capped at 1MB
    /// before reaching here, so this is at most a few thousand small tuples.
    fn get_elements(&self) -> Vec<(String, HashMap<String, String>)> {
        self.elements.clone()
    }
}

#[pyfunction]
fn load_page(html: String) -> ParsedPage {
    let document = kuchikiki::parse_html().one(html);

    let anchor_links = document
        .select("a")
        .unwrap()
        .filter_map(|node| {
            let attributes = node.attributes.borrow();
            let href = attributes.get("href")?.to_string();
            drop(attributes);
            let text = node
                .as_node()
                .first_child()
                .and_then(|child| child.as_text().map(|t| t.borrow().clone()))
                .unwrap_or_default();
            Some((href, text))
        })
        .collect();

    let mut link_attributes = Vec::new();
    let mut link_elements = Vec::new();
    let mut elements = Vec::new();
    for node in document.select("*").unwrap() {
        let tag = node.name.local.to_string();
        let attributes = node.attributes.borrow();
        let attrs_map: HashMap<String, String> = attributes
            .map
            .iter()
            .map(|(name, attr)| (name.local.to_string(), attr.value.clone()))
            .collect();

        for attr in LINK_ATTRS {
            if let Some(value) = attributes.get(attr) {
                link_attributes.push(value.to_string());
                link_elements.push((tag.clone(), value.to_string(), attrs_map.clone()));
            }
        }
        if let Some(value) = attributes.get("data-href") {
            link_attributes.push(value.to_string());
            link_elements.push((tag.clone(), value.to_string(), attrs_map.clone()));
        }

        drop(attributes);
        elements.push((tag, attrs_map));
    }

    let script_contents = document
        .select("script")
        .unwrap()
        .map(|node| node.as_node().to_string())
        .collect();

    let meta_tags = document
        .select("meta")
        .unwrap()
        .filter_map(|node| {
            let attributes = node.attributes.borrow();
            let key = attributes
                .get("name")
                .or_else(|| attributes.get("property"))?
                .to_string();
            let content = attributes.get("content").unwrap_or("").to_string();
            Some((key, content))
        })
        .collect();

    let json_ld = get_json_ld(&document);

    let anchor_text_fragments = document
        .select("a")
        .unwrap()
        .map(|node| {
            let attributes = node.attributes.borrow();
            let href = attributes.get("href").map(|v| v.to_string());
            drop(attributes);
            let fragments = node
                .as_node()
                .descendants()
                .text_nodes()
                .map(|text_node| text_node.borrow().to_string())
                .collect();
            (href, fragments)
        })
        .collect();

    let script_texts = document
        .select("script")
        .unwrap()
        .map(|node| {
            let attributes = node.attributes.borrow();
            let src = attributes.get("src").unwrap_or("").to_string();
            drop(attributes);
            let text = node
                .as_node()
                .children()
                .text_nodes()
                .map(|text_node| text_node.borrow().to_string())
                .collect::<Vec<_>>()
                .join("");
            (text, src)
        })
        .collect();

    ParsedPage {
        anchor_links,
        anchor_text_fragments,
        link_attributes,
        link_elements,
        script_contents,
        script_texts,
        meta_tags,
        json_ld,
        elements,
    }
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
    m.add_function(wrap_pyfunction!(strip_buttons_and_get_text, m)?)?;
    m.add_function(wrap_pyfunction!(load_page, m)?)?;
    m.add_class::<GetSentencesResult>()?;
    m.add_class::<ParsedPage>()?;
    Ok(())
}

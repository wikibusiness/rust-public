mod text_nodes;
mod utils;

use kuchiki::{iter::NodeIterator, traits::TendrilSink};
use linkify::{LinkFinder, LinkKind};
use pyo3::prelude::{pyclass, pyfunction, pymodule, wrap_pyfunction, Bound, PyModule, PyModuleMethods, PyResult};
use rayon::prelude::*;
use regex::RegexBuilder;
use std::collections::HashMap;
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
    m.add_function(wrap_pyfunction!(get_href_attributes, m)?)?;
    m.add_function(wrap_pyfunction!(get_alternate_links, m)?)?;
    m.add_function(wrap_pyfunction!(get_lang, m)?)?;
    m.add_function(wrap_pyfunction!(get_meta_titles, m)?)?;
    m.add_class::<GetSentencesResult>()?;
    Ok(())
}

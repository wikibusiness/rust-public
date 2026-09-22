//! Regression net for swapping out the internal HTML library (already used
//! once for the kuchiki -> kuchikiki swap): every public pyfunction gets run
//! against ~45 real crawled pages
//! pulled from the `htmls-production` R2 bucket (real crawl output, not
//! synthetic markup -- domains picked for size/language/TLD spread, see
//! tests/fixtures/) and the combined output is diffed against a checked-in
//! golden file per fixture.
//!
//! `cargo test` compares against the committed golden files. After an
//! intentional behavior change (e.g. a new HTML parser serializes tags
//! slightly differently), review the failing diffs, and if they're all
//! expected, regenerate with:
//!
//!   BLESS=1 cargo test --no-default-features golden_tests::
//!
//! then diff `git diff tests/golden` before committing -- that diff *is* the
//! behavioral change under review, so a one-line output tweak should produce
//! a one-line diff across the affected fixtures, not a wholesale rewrite.
use super::*;
use std::fmt::Write as _;

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

// Never matches real crawled text -- lets get_sentences/get_markdown's
// stop_word cleanup run without stripping anything, so these tests are
// exercising parsing/extraction, not the stop-word regex.
const NEVER_MATCHES: &str = "\u{e000}";

fn sorted_map_lines<V: std::fmt::Debug>(map: &HashMap<String, V>) -> Vec<String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|k| format!("  {k:?}: {:?}", map[k]))
        .collect()
}

fn render_fixture_report(html: &str) -> String {
    let mut out = String::new();

    macro_rules! section {
        ($name:expr, $body:expr) => {
            writeln!(out, "=== {} ===", $name).unwrap();
            writeln!(out, "{}", $body).unwrap();
            writeln!(out).unwrap();
        };
    }

    let lang = get_lang(html.to_string()).unwrap();
    section!("get_lang", lang);

    let meta_titles = get_meta_titles(html.to_string()).unwrap();
    section!("get_meta_titles", sorted_map_lines(&meta_titles).join("\n"));

    let alternate_links = get_alternate_links(html.to_string()).unwrap();
    section!(
        "get_alternate_links",
        sorted_map_lines(&alternate_links).join("\n")
    );

    let hrefs = get_href_attributes(html.to_string()).unwrap();
    section!("get_href_attributes", format!("{hrefs:?}"));

    let links = get_links(html.to_string()).unwrap();
    section!("get_links", format!("{links:?}"));

    let emails = get_emails(html.to_string()).unwrap();
    section!("get_emails", format!("{emails:?}"));

    let sentences =
        get_sentences(html.to_string(), NEVER_MATCHES, false, false, true, None).unwrap();
    section!(
        "get_sentences.descriptions",
        format!("{:?}", sentences.descriptions)
    );
    section!("get_sentences.h", sorted_map_lines(&sentences.h).join("\n"));
    section!("get_sentences.json_ld", format!("{:?}", sentences.json_ld));
    section!("get_sentences.keywords", sentences.keywords);
    section!("get_sentences.other", format!("{:?}", sentences.other));
    section!("get_sentences.p", format!("{:?}", sentences.p));
    section!(
        "get_sentences.meta_titles",
        sorted_map_lines(&sentences.meta_titles).join("\n")
    );
    section!(
        "get_sentences.href_attributes",
        format!("{:?}", sentences.href_attributes)
    );

    // The actual point of these fields: they must match a fresh standalone
    // parse of the same HTML exactly, not just look reasonable.
    assert_eq!(
        sentences.meta_titles, meta_titles,
        "get_sentences.meta_titles diverged from standalone get_meta_titles"
    );
    assert_eq!(
        sentences.href_attributes, hrefs,
        "get_sentences.href_attributes diverged from standalone get_href_attributes"
    );
    section!(
        "get_sentences.text_nodes",
        format!("{:?}", sentences.text_nodes)
    );

    let markdown = get_markdown(html.to_string(), NEVER_MATCHES, false, false).unwrap();
    section!("get_markdown", markdown);

    let extracted = extract_text_py(html.to_string(), None);
    section!("extract_text", sorted_map_lines(&extracted).join("\n"));

    let title_html = tag_html_contents(html.to_string(), "title".to_string()).unwrap();
    section!("tag_html_contents(title)", title_html);

    let contents = html_contents(html.to_string()).unwrap();
    section!("html_contents", contents);

    let page = load_page(html.to_string());
    section!("ParsedPage.get_anchor_links", format!("{:?}", page.get_anchor_links()));
    section!("ParsedPage.get_link_attributes", format!("{:?}", page.get_link_attributes()));
    let mut link_elements: Vec<(String, String, Vec<(String, String)>)> = page
        .get_link_elements()
        .into_iter()
        .map(|(tag, href, attrs)| {
            let mut attrs: Vec<(String, String)> = attrs.into_iter().collect();
            attrs.sort();
            (tag, href, attrs)
        })
        .collect();
    link_elements.sort();
    section!("ParsedPage.get_link_elements (sorted)", format!("{link_elements:?}"));
    section!("ParsedPage.get_script_contents", format!("{:?}", page.get_script_contents()));
    section!("ParsedPage.get_meta_tags", format!("{:?}", page.get_meta_tags()));
    section!("ParsedPage.get_json_ld", format!("{:?}", page.get_json_ld()));
    section!(
        "ParsedPage.get_anchor_text_fragments",
        format!("{:?}", page.get_anchor_text_fragments())
    );
    section!("ParsedPage.get_script_texts", format!("{:?}", page.get_script_texts()));
    // HashMap's Debug order isn't stable across process runs (hash-seed
    // randomized) -- render attrs sorted by key so this golden file doesn't
    // spuriously diff on every rerun.
    let mut elements: Vec<(String, Vec<(String, String)>)> = page
        .get_elements()
        .into_iter()
        .map(|(tag, attrs)| {
            let mut attrs: Vec<(String, String)> = attrs.into_iter().collect();
            attrs.sort();
            (tag, attrs)
        })
        .collect();
    elements.sort();
    section!("ParsedPage.get_elements (sorted)", format!("{elements:?}"));

    out
}

fn run_fixture(name: &str) {
    let html_path = format!("{FIXTURES_DIR}/{name}.html");
    let html = std::fs::read_to_string(&html_path)
        .unwrap_or_else(|e| panic!("failed to read fixture {html_path}: {e}"));

    let actual = render_fixture_report(&html);

    let golden_path = format!("{GOLDEN_DIR}/{name}.golden");
    if std::env::var_os("BLESS").is_some() {
        std::fs::write(&golden_path, &actual)
            .unwrap_or_else(|e| panic!("failed to write golden file {golden_path}: {e}"));
        return;
    }

    let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
        panic!(
            "missing golden file {golden_path} ({e}) -- run `BLESS=1 cargo test --no-default-features golden_tests::` to create it"
        )
    });

    assert_eq!(
        actual, expected,
        "\noutput for fixture '{name}' changed -- review the diff above; if it's an \
         intentional behavior change, rerun with BLESS=1 and commit the updated golden file"
    );
}

macro_rules! golden_test {
    ($test_name:ident, $fixture:expr) => {
        #[test]
        fn $test_name() {
            run_fixture($fixture);
        }
    };
}

golden_test!(fixture_d_00_3_net, "00-3.net");
golden_test!(fixture_d_00_al, "00.al");
golden_test!(fixture_d_00035666_xyz, "00035666.xyz");
golden_test!(fixture_d_00036268_xyz, "00036268.xyz");
golden_test!(fixture_d_0011_lt, "0011.lt");
golden_test!(fixture_d_005_hu, "005.hu");
golden_test!(fixture_d_005149_cn, "005149.cn");
golden_test!(fixture_d_00608_cc, "00608.cc");
golden_test!(fixture_d_007_ru, "007.ru");
golden_test!(fixture_d_007gameapk_com_br, "007gameapk.com.br");
golden_test!(fixture_d_007push_com_tw, "007push.com.tw");
golden_test!(fixture_d_0084288_xyz, "0084288.xyz");
golden_test!(fixture_d_00853six_cc, "00853six.cc");
golden_test!(fixture_d_008635_com, "008635.com");
golden_test!(fixture_d_0086parts_com, "0086parts.com");
golden_test!(fixture_d_0090_be, "0090.be");
golden_test!(fixture_d_00fffa_cn, "00fffa.cn");
golden_test!(fixture_d_00olr_cn, "00olr.cn");
golden_test!(fixture_d_01_magazin_ru, "01-magazin.ru");
golden_test!(fixture_d_01050_com, "01050.com");
golden_test!(fixture_d_010fotograaf_nl, "010fotograaf.nl");
golden_test!(
    fixture_d_010loodgieterrotterdam_nl,
    "010loodgieterrotterdam.nl"
);
golden_test!(fixture_d_010rotterdam_nl, "010rotterdam.nl");
golden_test!(fixture_d_010storage_nl, "010storage.nl");
golden_test!(fixture_d_01100111_com, "01100111.com");
golden_test!(fixture_d_012_de, "012.de");
golden_test!(fixture_d_01293_cn, "01293.cn");
golden_test!(fixture_d_014_fr, "014.fr");
golden_test!(fixture_d_0140_jp, "0140.jp");
golden_test!(fixture_d_014510_xyz, "014510.xyz");
golden_test!(fixture_d_01483_or_jp, "01483.or.jp");
golden_test!(fixture_d_01490169c_app, "01490169c.app");
golden_test!(fixture_d_014960_net, "014960.net");
golden_test!(fixture_d_01enter_de, "01enter.de");
golden_test!(fixture_d_01pompage_fr, "01pompage.fr");
golden_test!(fixture_d_020edu_net, "020edu.net");
golden_test!(fixture_d_020zuoche_net, "020zuoche.net");
golden_test!(fixture_d_0269_jp, "0269.jp");
golden_test!(fixture_d_029_114_cn, "029-114.cn");
golden_test!(fixture_d_02diagimmo_fr, "02diagimmo.fr");
golden_test!(fixture_d_02port_net, "02port.net");
golden_test!(fixture_d_02rf_ru, "02rf.ru");
golden_test!(fixture_d_02safoo_com, "02safoo.com");
golden_test!(fixture_d_02stroika_ru, "02stroika.ru");
golden_test!(fixture_d_030023612_xyz, "030023612.xyz");

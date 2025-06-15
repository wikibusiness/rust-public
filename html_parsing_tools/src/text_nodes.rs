use lazy_static::lazy_static;
use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use unicode_general_category::{get_general_category, GeneralCategory};

const OPEN_PUNCTUATION_CATEGORIES: [GeneralCategory; 2] = [
    GeneralCategory::InitialPunctuation,
    GeneralCategory::OpenPunctuation,
];
const CLOSE_PUNCTUATION_CATEGORIES: [GeneralCategory; 2] = [
    GeneralCategory::FinalPunctuation,
    GeneralCategory::ClosePunctuation,
];
const OPEN_PUNCTUATION_CHARS: [char; 2] = ['¡', '¿'];
const OPEN_PUNCTUATION_CHARS_EXTENDED: [char; 4] = ['¡', '¿', ',', ':'];
const CLOSE_PUNCTUATION_CHARS: [char; 7] = ['.', ',', ';', ':', '!', '?', '…'];

lazy_static! {
    static ref TEXT_LINES_RE: Regex = RegexBuilder::new("\n|[^\n]+")
        .build()
        .expect("Invalid Regex");
}

lazy_static! {
    static ref WORD_WITH_DOT_RE: Regex = RegexBuilder::new(r"^\.([\p{L}\p{N}]*)")
        .case_insensitive(true)
        .build()
        .expect("Invalid Regex");
}

lazy_static! {
    static ref WORDS_WITH_DOT: HashSet<String> = {
        let mut words: HashSet<String> =
            HashSet::from(["net", "travis", "git", "gitignore", "com", "gov", "org"])
                .iter()
                .map(|s| s.to_string())
                .collect();
        words.extend(rust_iso3166::ALL_ALPHA2.iter().map(|c| c.to_lowercase()));
        words
    };
}

lazy_static! {
    static ref SENTENCE_SPLIT_RE: Regex = RegexBuilder::new(r"\||\p{Zs}\-|\-\p{Zs}")
        .build()
        .expect("Invalid Regex");
}

// called form_text_nodes in Python
pub fn group_text_nodes(sentences: &[String], min_split: Option<i32>) -> Option<Vec<Vec<String>>> {
    if sentences.is_empty() {
        return None;
    }

    let mut text_nodes: Vec<Vec<String>> = vec![];
    let mut node: Vec<String> = vec![];
    let mut split_count = 0;

    let min_split = min_split.unwrap_or(select_min_split(sentences));
    let text_elements = separate_line_breakers(sentences);
    let text_elements_length = text_elements.len();

    for (ix, text) in text_elements.into_iter().enumerate() {
        let text_cleaned = text.trim();

        if text == "\n" {
            split_count += 1;
        } else if !(text_cleaned.is_empty() | text_cleaned.starts_with('<')) {
            node.push(text);
            split_count = 0;
        }

        if (split_count >= min_split) | (ix == text_elements_length - 1) {
            if node.is_empty() {
                continue;
            }

            if min_split == 0 {
                text_nodes.push(node);
            } else {
                text_nodes.push(regroup_node(&node));
            }

            node = vec![];
            split_count = 0;
        }
    }
    Some(text_nodes)
}

fn separate_line_breakers(sentences: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = vec![];

    for sentence in sentences {
        if sentence.is_empty() {
            continue;
        }
        let mut sentence_string = sentence.to_string();
        if sentence_string.contains('\r') {
            sentence_string = sentence_string.replace('\r', "\n");
        }
        for line in TEXT_LINES_RE.find_iter(&sentence_string) {
            lines.push(line.as_str().to_string());
        }
    }
    lines
}

fn select_min_split(sentences: &[String]) -> i32 {
    let mut breaks_counts: Vec<i32> = vec![];
    let mut breaks_count = 0;
    let mut counter_started = false;

    for sentence in sentences {
        if sentence.is_empty() {
            continue;
        }

        if counter_started {
            if *sentence == "\n" {
                breaks_count += 1;
            } else if !sentence.trim().is_empty() & (breaks_count > 0) {
                breaks_counts.push(breaks_count);
                breaks_count = 0;
            }
        } else if !sentence.trim().is_empty() {
            counter_started = true;
        }
    }
    if breaks_counts.is_empty() {
        return 1;
    }
    breaks_counts.sort();
    let mid = breaks_counts.len() / 2;
    let median = breaks_counts[mid];

    for count in breaks_counts {
        if count > median {
            return median + 1;
        }
    }
    median
}

fn split_sentence(
    sentence: &str,
    split_re: &Regex,
    split_words: Option<&HashSet<&str>>,
) -> Vec<String> {
    let mut result: Vec<String> = vec![];

    for mut part in split_re.split(sentence) {
        part = part.trim();
        if !part.is_empty() {
            if let Some(split_words) = &split_words {
                result.extend(split_by_words(part, split_words));
            } else {
                result.push(part.to_string());
            }
        }
    }

    result
}

fn split_by_words(sentence: &str, split_words: &HashSet<&str>) -> Vec<String> {
    let mut result: Vec<String> = vec![];
    let mut temp: Vec<String> = vec![];

    for word in sentence.split_whitespace() {
        if split_words.contains(word.to_lowercase().as_str()) {
            if !temp.is_empty() {
                result.push(temp.join(" "));
                temp = vec![];
            }
            continue;
        }
        temp.push(word.to_string());
    }
    if !temp.is_empty() {
        result.push(temp.join(" "));
    }
    result
}

fn join_sentences(sentences: &[&str]) -> String {
    let mut words: Vec<String> = vec![];
    let dot_marker = "[\\DOT]";
    let dot_marker_with_dot = format!("{}.", dot_marker);
    let mut marked = false;

    for word_as_ref in sentences.join(" ").split_whitespace() {
        let mut word = word_as_ref.to_string();
        if word.starts_with('.') & (word.len() > 1) {
            if let Some(captures) = WORD_WITH_DOT_RE.captures(&word) {
                if let Some(wordbase) = captures.get(1) {
                    let wordbase = wordbase.as_str().to_lowercase();
                    if wordbase.parse::<f64>().is_ok() | WORDS_WITH_DOT.contains(&wordbase) {
                        word = format!("{}{}", dot_marker, word);
                        marked = true;
                    }
                }
            }
        }
        words.push(word);
    }

    let mut text = words.join(" ");

    for ch in HashSet::<char>::from_iter(text.chars()) {
        let unicode_category = get_general_category(ch);
        let error_case: String;

        if OPEN_PUNCTUATION_CATEGORIES.contains(&unicode_category)
            | OPEN_PUNCTUATION_CHARS.contains(&ch)
        {
            error_case = format!("{} ", ch);
        } else if CLOSE_PUNCTUATION_CATEGORIES.contains(&unicode_category)
            | CLOSE_PUNCTUATION_CHARS.contains(&ch)
        {
            error_case = format!(" {}", ch);
        } else {
            continue;
        }

        if text.contains(&error_case) {
            text = text.replace(&error_case, ch.to_string().as_str());
        }
    }

    if marked {
        text = text.replace(&dot_marker_with_dot, ".");
    }

    text
}

fn regroup_node(node: &[String]) -> Vec<String> {
    let length = node.len();
    if length == 1 {
        return split_sentence(node.get(0).unwrap(), &SENTENCE_SPLIT_RE, None);
    }
    let mut regrouped: Vec<String> = vec![];
    let mut group: Vec<&str> = vec![node.get(0).unwrap().as_ref()];

    for ix in 1..length {
        let left = node.get(ix - 1).unwrap().chars().last().unwrap();
        let right = node.get(ix).unwrap().chars().nth(0).unwrap();
        let left_category = get_general_category(left);
        let right_category = get_general_category(right);

        let tied = {
            left.is_whitespace()
                | right.is_whitespace()
                | OPEN_PUNCTUATION_CATEGORIES.contains(&left_category)
                | CLOSE_PUNCTUATION_CATEGORIES.contains(&right_category)
                | OPEN_PUNCTUATION_CHARS_EXTENDED.contains(&left)
                | CLOSE_PUNCTUATION_CHARS.contains(&right)
        };

        if tied {
            group.push(node.get(ix).unwrap());
        } else {
            regrouped.extend(split_sentence(
                join_sentences(&group).as_str(),
                &SENTENCE_SPLIT_RE,
                None,
            ));
            group = vec![node.get(ix).unwrap().as_ref()];
        }
    }

    regrouped.extend(split_sentence(
        join_sentences(&group).as_str(),
        &SENTENCE_SPLIT_RE,
        None,
    ));

    regrouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_to_string(sentences: &[&str]) -> Vec<String> {
        sentences.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_separate_line_breakers() {
        let sentences = convert_to_string(&["", " ", "", "Hello\n\rWorld\n\n", "\n\n\r", "\n!"]);
        let result = separate_line_breakers(&sentences);
        assert_eq!(
            result,
            [" ", "Hello", "\n", "\n", "World", "\n", "\n", "\n", "\n", "\n", "\n", "!"],
        );
        assert_eq!(separate_line_breakers(&["\r".to_string()]), ["\n"]);
        assert_eq!(separate_line_breakers(&["hello".to_string()]), ["hello"]);
    }

    #[test]
    fn test_select_min_split() {
        let sentences = convert_to_string(&["1", "\n", "", "2", "\n", "3", "\n", "", "4"]);
        assert_eq!(select_min_split(&sentences), 1);

        let sentences =
            convert_to_string(&["1", "\n", "\n", "2", "\n", "\n", "3", "\n", "\n", "4"]);
        assert_eq!(select_min_split(&sentences), 2);

        let sentences =
            convert_to_string(&["1", "\n", "\n", "2", "\n", "\n", "\n", "3", "\n", "\n", "4"]);
        assert_eq!(select_min_split(&sentences), 3);

        let sentences =
            convert_to_string(&["\n", "\n", "\n", "1", "2", "\n", "\n", "\n", "\n", "3"]);
        assert_eq!(select_min_split(&sentences), 4);

        let sentences = convert_to_string(&["1", "2", "3", "4"]);
        assert_eq!(select_min_split(&sentences), 1);

        let sentences =
            convert_to_string(&["\n", "\n", "\n", "1", "2", "\n", "\n", "\n", "\n", "\n"]);
        assert_eq!(select_min_split(&sentences), 1);

        let sentences = convert_to_string(&["\n", "\n", "\n", "\n", "\n", "\n", "\n", "\n", "\n"]);
        assert_eq!(select_min_split(&sentences), 1);

        let sentences = vec![];
        assert_eq!(select_min_split(&sentences), 1);
    }

    #[test]
    fn test_split_sentence() {
        assert_eq!(
            split_sentence("about | hello | world", &SENTENCE_SPLIT_RE, None),
            vec!["about", "hello", "world"],
        );

        assert_eq!(
            split_sentence(
                "About us | Revenue Intelligence for ABM | Ocean.io | ",
                &SENTENCE_SPLIT_RE,
                Some(&HashSet::from(["about", "for"])),
            ),
            vec!["us", "Revenue Intelligence", "ABM", "Ocean.io"],
        );

        assert_eq!(
            split_sentence(
                "ocean CRM",
                &SENTENCE_SPLIT_RE,
                Some(&HashSet::from(["about", "for"])),
            ),
            vec!["ocean CRM"],
        );

        let empty_vector: Vec<String> = vec![];
        assert_eq!(
            split_sentence(
                "ABOUT  for",
                &SENTENCE_SPLIT_RE,
                Some(&HashSet::from(["about", "for"])),
            ),
            empty_vector,
        );
    }

    #[test]
    fn test_join_sentences() {
        let sentences = vec!["tel.", ":", "+390441234567"];
        assert_eq!(join_sentences(&sentences), "tel.: +390441234567");

        let sentences = vec![
            "Vinissimus",
            ":",
            "Comprar vino online",
            "-",
            "Venta de vinos de España y resto del mundo",
            ".",
        ];
        assert_eq!(
            join_sentences(&sentences),
            "Vinissimus: Comprar vino online - Venta de vinos de España y resto del mundo."
        );

        let sentences = vec![
            "Polina",
            "\n\r\n\t     ",
            " Voroshilova\t",
            ", email:",
            "voroshilova@gmail.com",
        ];
        assert_eq!(
            join_sentences(&sentences),
            "Polina Voroshilova, email: voroshilova@gmail.com"
        );

        let sentences = vec![
            "Transporte",
            "(",
            "información y gastos de envío",
            ")",
            " ",
            "*",
            "¿",
            "Quiénes somos",
            "\n",
            "?",
        ];
        assert_eq!(
            join_sentences(&sentences),
            "Transporte (información y gastos de envío) * ¿Quiénes somos?"
        );

        let sentences = vec![
            "F#,",
            ".NET,",
            "C# ect.",
            "Please, add",
            ".gitignore and",
            ".travis.yml",
            "for all",
            ".dk",
            "and .com domains.",
        ];
        assert_eq!(
            join_sentences(&sentences),
            "F#, .NET, C# ect. Please, add .gitignore and .travis.yml for all .dk and .com domains."
        );

        let empty_vector: Vec<&str> = vec![];
        assert_eq!(join_sentences(&empty_vector), "");
    }

    #[test]
    fn test_regroup_node() {
        assert_eq!(
            regroup_node(&["Más información y precios".to_string()]),
            ["Más información y precios"],
        );

        let sentences = convert_to_string(&[
            "Toni Vicens, ",
            "Fundador de Vinissimus ",
            "tvicens@vinissimus.com",
        ]);

        assert_eq!(
            regroup_node(&sentences),
            ["Toni Vicens, Fundador de Vinissimus tvicens@vinissimus.com"],
        );

        let sentences = convert_to_string(&[
            "Inicio",
            "Quiénes somos",
            "Contáctenos",
            "Comprar vino por Internet",
            "Blog",
            "Transporte (información y gastos de envío)",
            "Ayuda",
            "Uso de cookies",
        ]);

        assert_eq!(
            regroup_node(&sentences),
            [
                "Inicio",
                "Quiénes somos",
                "Contáctenos",
                "Comprar vino por Internet",
                "Blog",
                "Transporte (información y gastos de envío)",
                "Ayuda",
                "Uso de cookies"
            ],
        );

        let sentences = convert_to_string(&[
            "vinissimus.com",
            " (español)",
            "vinissimus.co.uk",
            " (english)",
            "vinissimus.fr",
            " (français)",
            "vinissimus.it",
            " (italiano)",
            "hispavinus.de",
            " (deutsch)",
        ]);

        assert_eq!(
            regroup_node(&sentences),
            [
                "vinissimus.com (español)",
                "vinissimus.co.uk (english)",
                "vinissimus.fr (français)",
                "vinissimus.it (italiano)",
                "hispavinus.de (deutsch)"
            ],
        );

        let sentences = convert_to_string(&[
            "Vinissimus le recuerda que ",
            "no está permitida la venta de bebidas alcohólicas",
            " a menores de 18 años",
            " y le recomienda ",
            "consumirlas con moderación",
            ", que es como mejor se disfrutan.",
        ]);

        assert_eq!(
            regroup_node(&sentences),
            ["Vinissimus le recuerda que no está permitida la venta de bebidas alcohólicas a menores de 18 años y le recomienda consumirlas con moderación, que es como mejor se disfrutan."],
        );
    }

    #[test]
    fn test_group_text_nodes() {
        let sentences = convert_to_string(&[
            "Más información y precios",
            " ",
            "\n",
            "\n",
            "Gastos de envío",
            "\n",
            "\n",
            "\n",
            "\nToni Vicens, ",
            "\n",
            "Fundador de Vinissimus ",
            "\n",
            "tvicens@vinissimus.com",
            "\n",
            "\n",
            "\n",
            "Inicio",
            "\n",
            "Quiénes somos",
            "\n",
            "Contáctenos",
            "\n",
            "Comprar vino por Internet",
            "\n",
            "Blog",
            "\n",
            "Transporte (información y gastos de envío)",
            "\n",
            "Ayuda",
            "\n",
            "Uso de cookies",
            "\n",
            "\n",
            "\n",
            "\nNuestras webs\n                   ",
            "\n",
            "\n",
            "Venta de vino italiano",
            "\n",
            "\n",
            "\n",
            "\n",
            "vinissimus.com",
            " (español)",
            "\n",
            "vinissimus.co.uk",
            " (english)",
            "\n",
            "vinissimus.fr",
            " (français)",
            "\n",
            "vinissimus.it",
            " (italiano)",
            "\n",
            "hispavinus.de",
            " (deutsch)",
            "\n",
            "\n",
            "\nVinissimus le recuerda que ",
            "no está permitida la venta de bebidas alcohólicas",
            " a menores de 18 años",
            " y le recomienda ",
            "consumirlas con moderación",
            ", que es como mejor se disfrutan.\n                       ",
            "\n",
            "\n",
            "Follow @vinissimus",
            "\n",
            "\n",
            "\n",
        ]);

        let result = group_text_nodes(&sentences, None);
        assert_eq!(
            result.unwrap(),
            [
                vec!["Más información y precios"],
                vec!["Gastos de envío"],
                vec!["Toni Vicens, Fundador de Vinissimus tvicens@vinissimus.com"],
                vec![
                    "Inicio",
                    "Quiénes somos",
                    "Contáctenos",
                    "Comprar vino por Internet",
                    "Blog",
                    "Transporte (información y gastos de envío)",
                    "Ayuda",
                    "Uso de cookies",
                ],
                vec!["Nuestras webs"],
                vec!["Venta de vino italiano"],
                vec![
                    "vinissimus.com (español)",
                    "vinissimus.co.uk (english)",
                    "vinissimus.fr (français)",
                    "vinissimus.it (italiano)",
                    "hispavinus.de (deutsch)",
                ],
                vec!["Vinissimus le recuerda que no está permitida la venta de bebidas alcohólicas a menores de 18 años y le recomienda consumirlas con moderación, que es como mejor se disfrutan."],
                vec!["Follow @vinissimus"],
            ],
        );

        let result = group_text_nodes(&sentences, Some(99999));
        assert_eq!(
            result.unwrap(),
            [
                [
                    "Más información y precios",
                    "Gastos de envío",
                    "Toni Vicens, Fundador de Vinissimus tvicens@vinissimus.com",
                    "Inicio",
                    "Quiénes somos",
                    "Contáctenos",
                    "Comprar vino por Internet",
                    "Blog",
                    "Transporte (información y gastos de envío)",
                    "Ayuda",
                    "Uso de cookies",
                    "Nuestras webs",
                    "Venta de vino italiano",
                    "vinissimus.com (español)",
                    "vinissimus.co.uk (english)",
                    "vinissimus.fr (français)",
                    "vinissimus.it (italiano)",
                    "hispavinus.de (deutsch)",
                    "Vinissimus le recuerda que no está permitida la venta de bebidas alcohólicas a menores de 18 años y le recomienda consumirlas con moderación, que es como mejor se disfrutan.",
                    "Follow @vinissimus",
                ],
            ],
        );
        assert_eq!(
            group_text_nodes(&["hello".to_string()], None).unwrap(),
            [["hello"]],
        );
        assert_eq!(group_text_nodes(&[], None), None,);
    }
}

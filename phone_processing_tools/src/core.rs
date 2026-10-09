use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use prost::Message;
use regex::Regex;
use rlibphonenumber::{PhoneMetadataCollection, PhoneNumberFormat, PhoneNumberUtil, Region};
use serde::Deserialize;

use crate::{GroupError, ParsedLink, PhoneRecord};

pub const URL_PREFIXES: &[&str] = &[
    "tel",
    "callto",
    "facetime",
    "sms",
    "skype",
    "whatsapp",
    "fax",
    "https://wa.me/",
    "https://api.whatsapp.com/",
];
const SCHEMES: &[&str] = &[
    "tel", "callto", "facetime", "sms", "skype", "whatsapp", "fax",
];
const PHONE_WORDS: &[&str] = &[
    "cell phone",
    "cellphone",
    "cell",
    "mobile",
    "mobil",
    "movil",
    "mobilnye",
    "cp",
    "mbl",
    "mob",
    "mp",
    "m",
    "landline",
    "telefon",
    "telefono",
    "telephone",
    "phone",
    "fono",
    "fon",
    "call",
    "ddi",
    "tel",
    "tlf",
    "t",
    "p",
    "ph",
];
const FAX_WORDS: &[&str] = &["telefax", "fax", "faks", "f", "facsimile", "faksimile"];

#[derive(Deserialize)]
struct Normalization {
    versions: BTreeMap<String, String>,
    transliterations: HashMap<u32, String>,
    symbols: HashMap<u32, String>,
    numeric: HashSet<u32>,
    decimal: HashSet<u32>,
}

static NORMALIZATION: LazyLock<Normalization> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/normalization.json"))
        .expect("generated normalization data")
});
static UTIL: LazyLock<PhoneNumberUtil> = LazyLock::new(|| {
    let metadata = PhoneMetadataCollection::decode(&include_bytes!("../data/metadata.bin")[..])
        .expect("generated phone metadata");
    PhoneNumberUtil::new_for_metadata(metadata).expect("generated phone territories")
});
static NUMBERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+?[0-9 ]+").unwrap());
static YEARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]{4})\s?([0-9]{4})$").unwrap());

fn whitespace(character: char) -> bool {
    matches!(character, '\t'..='\r' | '\u{1c}'..='\u{20}')
}

pub fn versions() -> BTreeMap<String, String> {
    NORMALIZATION.versions.clone()
}

pub fn normalize(text: &str) -> String {
    let mut symbols = String::with_capacity(text.len());
    for character in text.chars() {
        if let Some(value) = NORMALIZATION.symbols.get(&(character as u32)) {
            symbols.extend(value.chars().filter(|c| !".-/".contains(*c)));
        } else if !".-/".contains(character) {
            symbols.push(character);
        }
    }
    let mut unwrapped = String::with_capacity(symbols.len());
    let mut characters = symbols.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '(' {
            let mut probe = characters.clone();
            let mut count = 0;
            while probe.peek().is_some_and(|c| {
                c.is_ascii_digit()
                    || matches!(*c, '+' | ' ')
                    || NORMALIZATION.numeric.contains(&(*c as u32))
            }) {
                probe.next();
                count += 1;
            }
            if count > 0 && probe.peek() == Some(&')') {
                for _ in 0..count {
                    unwrapped.push(characters.next().unwrap());
                }
                characters.next();
                continue;
            }
        }
        unwrapped.push(character);
    }
    let mut ascii = String::with_capacity(unwrapped.len());
    for character in unwrapped.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else if let Some(value) = NORMALIZATION.transliterations.get(&(character as u32)) {
            ascii.push_str(value);
        }
    }
    ascii
        .split(whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn clean_number(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_digit() || *c == '+' || NORMALIZATION.decimal.contains(&(*c as u32)))
        .collect()
}

fn within_bounds(text: &str) -> bool {
    let digits = text.bytes().filter(u8::is_ascii_digit).count();
    (6..=20).contains(&digits) && digits as f64 / text.len() as f64 >= 0.6
}

fn candidate(number: String, raw: String, kind: &str) -> PhoneRecord {
    PhoneRecord {
        number,
        raw: Some(vec![raw]),
        kind: kind.to_owned(),
        validated: Some(false),
        sources: None,
        country: None,
        primary: None,
    }
}

pub fn extract_text(text: &str, include_unknown: bool, max_year: u32) -> Vec<PhoneRecord> {
    let text = normalize(text);
    let mut result = Vec::new();
    let mut last_end = 0;
    for found in NUMBERS.find_iter(&text) {
        let raw = found.as_str().trim();
        if !within_bounds(raw) {
            continue;
        }
        let previous: String = text[last_end..found.start()]
            .chars()
            .filter(|c| c.is_ascii_alphabetic() || *c == ' ')
            .collect();
        let words: Vec<_> = previous.split_whitespace().rev().take(2).collect();
        last_end = found.end();
        let kind = if words.iter().any(|word| FAX_WORDS.contains(word)) {
            "fax"
        } else if raw.starts_with('+') || words.iter().any(|word| PHONE_WORDS.contains(word)) {
            "phone"
        } else {
            if !include_unknown {
                continue;
            }
            if let Some(years) = YEARS.captures(raw) {
                let start: u32 = years[1].parse().unwrap();
                let end: u32 = years[2].parse().unwrap();
                if start >= 1989 && end <= max_year && start <= end {
                    continue;
                }
            }
            "unknown"
        };
        result.push(candidate(clean_number(raw), raw.to_owned(), kind));
    }
    result
}

fn extract_values(values: Vec<String>, kind: &str) -> Vec<PhoneRecord> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        if !seen.insert(value.clone()) {
            continue;
        }
        let normalized = normalize(&value);
        for found in NUMBERS.find_iter(&normalized) {
            let raw = found.as_str().trim();
            if within_bounds(raw) {
                result.push(candidate(clean_number(raw), raw.to_owned(), kind));
            }
        }
    }
    result
}

pub fn extract_json(phones: Vec<String>, faxes: Vec<String>) -> Vec<PhoneRecord> {
    let mut result = extract_values(phones, "phone");
    result.extend(extract_values(faxes, "fax"));
    result
}

pub fn prepare_links(links: Vec<String>, lowercase: bool) -> Vec<(String, String)> {
    links
        .into_iter()
        .filter_map(|raw| {
            let raw = if lowercase { raw.to_lowercase() } else { raw };
            if raw.starts_with("https://wa.me/") || raw.starts_with("https://api.whatsapp.com/") {
                Some((raw.clone(), raw))
            } else if raw.starts_with("http") || raw.starts_with('/') {
                None
            } else {
                let url = raw.replace("://", ":");
                Some((raw, url))
            }
        })
        .collect()
}

fn plus(number: &str) -> String {
    if !number.is_empty() && !number.starts_with('+') {
        format!("+{}", number.trim())
    } else {
        number.to_owned()
    }
}

pub fn extract_links(links: Vec<ParsedLink>) -> Vec<PhoneRecord> {
    let mut result = Vec::new();
    for link in links {
        let whatsapp = link.original.starts_with("https://wa.me/")
            || link.original.starts_with("https://api.whatsapp.com/");
        let mut number = None;
        let mut kind = "phone";
        if whatsapp {
            let value = if link.host.as_deref() == Some("wa.me")
                && !["/", "/#", ""].contains(&link.path.as_str())
            {
                Some(link.path.replace('/', ""))
            } else {
                link.phone.clone()
            };
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                number = Some(plus(&value));
            }
        } else if SCHEMES.contains(&link.scheme.as_str()) {
            if link.scheme == "whatsapp" {
                number = link.phone.as_deref().map(plus);
            } else {
                number = Some(link.path);
                if link.scheme == "fax" {
                    kind = "fax";
                }
            }
        }
        if let Some(number) = number {
            result.extend(extract_values(vec![number], kind));
        }
    }
    result
}

fn parsed(
    number: &str,
    country: Option<&str>,
    international: bool,
) -> Option<(String, Option<String>)> {
    let country = country.and_then(|c| Region::from_code(&c.to_uppercase()).ok());
    let number = UTIL.parse(number, country).ok()?;
    if !UTIL.is_valid_number(&number) {
        return None;
    }
    let mode = if international {
        PhoneNumberFormat::International
    } else {
        PhoneNumberFormat::E164
    };
    Some((
        UTIL.format(&number, mode).into_owned(),
        UTIL.get_region_for_number(&number)
            .map(|c| c.to_string().to_lowercase()),
    ))
}

pub fn format_numbers(numbers: Vec<String>, international: bool) -> Vec<Option<String>> {
    let mut cache = HashMap::new();
    numbers
        .iter()
        .map(|raw| {
            let cleaned = clean_number(raw);
            cache
                .entry(cleaned.clone())
                .or_insert_with(|| parsed(&cleaned, None, international).map(|p| p.0))
                .clone()
        })
        .collect()
}

fn dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn frequency(numbers: impl Iterator<Item = String>) -> Vec<String> {
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut entries: Vec<(String, usize)> = Vec::new();
    for number in numbers {
        if let Some(index) = indices.get(&number) {
            entries[*index].1 += 1;
        } else {
            indices.insert(number.clone(), entries.len());
            entries.push((number, 1));
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    entries.into_iter().map(|p| p.0).collect()
}

pub fn validate(
    mut phones: Vec<PhoneRecord>,
    countries: Vec<String>,
    international: bool,
) -> Result<Vec<PhoneRecord>, GroupError> {
    for phone in &mut phones {
        if phone.validated != Some(true) || international {
            for country in std::iter::once(None).chain(countries.iter().map(|c| Some(c.as_str()))) {
                if !phone.number.starts_with('+') && country.is_none() {
                    continue;
                }
                if phone.raw.as_ref().is_none_or(|r| r.is_empty()) {
                    phone.raw = Some(vec![phone.number.clone()]);
                    phone.number = clean_number(&phone.number);
                }
                phone.validated = Some(false);
                if let Some((number, region)) = parsed(&phone.number, country, international) {
                    phone.number = number;
                    phone.validated = Some(true);
                    if region.is_some() {
                        phone.country = region;
                    }
                    break;
                }
            }
        }
    }
    phones.sort_by_key(|phone| std::cmp::Reverse(phone.validated == Some(true)));
    let mut groups: Vec<PhoneRecord> = Vec::new();
    let mut kinds: Vec<Vec<String>> = Vec::new();
    let mut indices: HashMap<String, usize> = HashMap::new();
    for phone in &phones {
        let suffix = phone.number.strip_prefix('0').unwrap_or(&phone.number);
        let number = groups
            .iter()
            .find(|p| p.number.ends_with(suffix))
            .map(|p| p.number.clone())
            .unwrap_or_else(|| phone.number.clone());
        if let Some(&index) = indices.get(&number) {
            kinds[index].push(phone.kind.clone());
            groups[index]
                .raw
                .as_mut()
                .ok_or(GroupError::CannotExtend)?
                .extend(phone.raw.clone().ok_or(GroupError::NotIterable)?);
            groups[index]
                .sources
                .as_mut()
                .ok_or(GroupError::CannotExtend)?
                .extend(phone.sources.clone().ok_or(GroupError::NotIterable)?);
        } else {
            indices.insert(number.clone(), groups.len());
            let mut grouped = phone.clone();
            grouped.number = number;
            grouped.primary = None;
            groups.push(grouped);
            kinds.push(vec![phone.kind.clone()]);
        }
    }
    let mut numbers = frequency(
        phones
            .iter()
            .filter(|p| p.validated == Some(true))
            .map(|p| p.number.clone()),
    );
    numbers.extend(frequency(
        phones
            .iter()
            .filter(|p| p.validated != Some(true) && indices.contains_key(&p.number))
            .map(|p| p.number.clone()),
    ));
    let mut output = Vec::new();
    for number in numbers {
        let index = *indices
            .get(&number)
            .ok_or_else(|| GroupError::MissingNumber(number.clone()))?;
        let mut phone = groups[index].clone();
        phone.kind = if kinds[index].iter().any(|k| k == "phone") {
            "phone"
        } else if kinds[index].iter().any(|k| k == "fax") {
            "fax"
        } else {
            "unknown"
        }
        .to_owned();
        phone.raw = Some(dedup(phone.raw.ok_or(GroupError::NotIterable)?));
        phone.sources = Some(dedup(phone.sources.ok_or(GroupError::NotIterable)?));
        output.push(phone);
    }
    Ok(output)
}

pub fn extract(
    text: String,
    json_phones: Vec<String>,
    json_faxes: Vec<String>,
    links: Vec<ParsedLink>,
    include_unknown: bool,
    max_year: u32,
) -> Result<Vec<PhoneRecord>, GroupError> {
    let mut phones = Vec::new();
    for (source, values) in [
        ("text", extract_text(&text, include_unknown, max_year)),
        ("json_ld", extract_json(json_phones, json_faxes)),
        ("link", extract_links(links)),
    ] {
        for mut phone in values {
            phone.sources = Some(vec![source.to_owned()]);
            phones.push(phone);
        }
    }
    validate(phones, vec![], false)
}

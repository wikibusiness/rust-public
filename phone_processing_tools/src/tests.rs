use crate::{core, PhoneRecord};

#[test]
fn normalization_and_classification() {
    assert_eq!(
        core::normalize("FAX: +４５ ３３６６ ３３６６"),
        "fax: +45 3366 3366"
    );
    let result = core::extract_text("Fax: +4533663366; 2015-2020; 1234567", true, 2027);
    assert_eq!(
        result
            .iter()
            .map(|p| (p.number.as_str(), p.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![("+4533663366", "fax"), ("1234567", "unknown")]
    );
}

#[test]
fn pinned_numbering_metadata() {
    assert_eq!(
        core::format_numbers(vec!["+453 366 3366".into(), "11111".into()], true),
        vec![Some("+45 33 66 33 66".into()), None]
    );
    assert_eq!(
        core::format_numbers(vec!["+39 02 12345678".into()], true),
        vec![Some("+39 02 1234 5678".into())]
    );
}

#[test]
fn ordered_merging() {
    let make = |kind: &str, source: &str| PhoneRecord {
        number: "+4533663366".into(),
        raw: Some(vec!["+4533663366".into()]),
        kind: kind.into(),
        validated: Some(false),
        sources: Some(vec![source.into()]),
        country: None,
        primary: Some(true),
    };
    let result = core::validate(
        vec![make("fax", "text"), make("phone", "link")],
        vec![],
        false,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].kind, "phone");
    assert_eq!(result[0].sources, Some(vec!["text".into(), "link".into()]));
    assert_eq!(result[0].primary, None);
}

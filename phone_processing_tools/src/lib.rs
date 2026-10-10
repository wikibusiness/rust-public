mod core;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use pyo3::exceptions::{PyAttributeError, PyKeyError, PyTypeError};
use pyo3::prelude::*;

#[pyclass(get_all, frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct PhoneRecord {
    pub number: String,
    pub raw: Option<Vec<String>>,
    pub kind: String,
    pub validated: Option<bool>,
    pub sources: Option<Vec<String>>,
    pub country: Option<String>,
    pub primary: Option<bool>,
}

#[pymethods]
impl PhoneRecord {
    #[new]
    fn new(
        number: String,
        raw: Option<Vec<String>>,
        kind: String,
        validated: Option<bool>,
        sources: Option<Vec<String>>,
        country: Option<String>,
        primary: Option<bool>,
    ) -> Self {
        Self {
            number,
            raw,
            kind,
            validated,
            sources,
            country,
            primary,
        }
    }
}

#[pyclass(get_all, frozen, from_py_object)]
#[derive(Clone, Debug)]
pub struct ParsedLink {
    pub original: String,
    pub scheme: String,
    pub host: Option<String>,
    pub path: String,
    pub phone: Option<String>,
}

#[pymethods]
impl ParsedLink {
    #[new]
    fn new(
        original: String,
        scheme: String,
        host: Option<String>,
        path: String,
        phone: Option<String>,
    ) -> Self {
        Self {
            original,
            scheme,
            host,
            path,
            phone,
        }
    }
}

#[derive(Debug)]
pub enum GroupError {
    NotIterable,
    CannotExtend,
    MissingNumber(String),
}

impl From<GroupError> for PyErr {
    fn from(value: GroupError) -> Self {
        match value {
            GroupError::NotIterable => PyTypeError::new_err("'NoneType' object is not iterable"),
            GroupError::CannotExtend => {
                PyAttributeError::new_err("'NoneType' object has no attribute 'extend'")
            }
            GroupError::MissingNumber(number) => PyKeyError::new_err(number),
        }
    }
}

#[pyfunction]
fn metadata_versions(py: Python<'_>) -> BTreeMap<String, String> {
    py.detach(core::versions)
}

#[pyfunction]
fn preprocess_texts(py: Python<'_>, texts: Vec<String>) -> Vec<String> {
    py.detach(|| texts.iter().map(|text| core::normalize(text)).collect())
}

#[pyfunction]
fn extract_text_phones(
    py: Python<'_>,
    text: String,
    include_unknown: bool,
    max_year: u32,
) -> Vec<PhoneRecord> {
    py.detach(|| core::extract_text(&text, include_unknown, max_year))
}

#[pyfunction]
fn extract_json_ld_phones(
    py: Python<'_>,
    phones: Vec<String>,
    faxes: Vec<String>,
) -> Vec<PhoneRecord> {
    py.detach(|| core::extract_json(phones, faxes))
}

#[pyfunction]
fn prepare_links(py: Python<'_>, links: Vec<String>, lowercase: bool) -> Vec<(String, String)> {
    py.detach(|| core::prepare_links(links, lowercase))
}

#[pyfunction]
fn extract_links_phones(py: Python<'_>, links: Vec<Py<ParsedLink>>) -> Vec<PhoneRecord> {
    let links = links
        .into_iter()
        .map(|link| link.borrow(py).clone())
        .collect();
    py.detach(|| core::extract_links(links))
}

#[pyfunction]
fn validate_phones(
    py: Python<'_>,
    phones: Vec<Py<PhoneRecord>>,
    countries: Vec<String>,
    national_format: bool,
) -> PyResult<Vec<PhoneRecord>> {
    let phones = phones
        .into_iter()
        .map(|phone| phone.borrow(py).clone())
        .collect();
    py.detach(|| core::validate(phones, countries, national_format))
        .map_err(Into::into)
}

#[pyfunction]
fn extract_phones(
    py: Python<'_>,
    text: String,
    phones: Vec<String>,
    faxes: Vec<String>,
    links: Vec<Py<ParsedLink>>,
    include_unknown: bool,
    max_year: u32,
) -> PyResult<Vec<PhoneRecord>> {
    let links = links
        .into_iter()
        .map(|link| link.borrow(py).clone())
        .collect();
    py.detach(|| core::extract(text, phones, faxes, links, include_unknown, max_year))
        .map_err(Into::into)
}

#[pyfunction]
fn format_numbers(
    py: Python<'_>,
    numbers: Vec<String>,
    national_format: bool,
) -> Vec<Option<String>> {
    py.detach(|| core::format_numbers(numbers, national_format))
}

#[pymodule]
fn phone_processing_tools(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PhoneRecord>()?;
    module.add_class::<ParsedLink>()?;
    module.add("URL_PREFIXES", core::URL_PREFIXES.to_vec())?;
    module.add_function(wrap_pyfunction!(metadata_versions, module)?)?;
    module.add_function(wrap_pyfunction!(preprocess_texts, module)?)?;
    module.add_function(wrap_pyfunction!(extract_text_phones, module)?)?;
    module.add_function(wrap_pyfunction!(extract_json_ld_phones, module)?)?;
    module.add_function(wrap_pyfunction!(prepare_links, module)?)?;
    module.add_function(wrap_pyfunction!(extract_links_phones, module)?)?;
    module.add_function(wrap_pyfunction!(validate_phones, module)?)?;
    module.add_function(wrap_pyfunction!(extract_phones, module)?)?;
    module.add_function(wrap_pyfunction!(format_numbers, module)?)?;
    Ok(())
}

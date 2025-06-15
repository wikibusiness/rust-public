use std::collections::HashMap;

use human_name::Name;
use pyo3::prelude::*;

#[pyfunction]
fn parse_name(name: String) -> PyResult<HashMap<String, String>> {
    let mut result = HashMap::new();

    let n = Name::parse(name.as_str());

    if n.is_none() {
        return Ok(result);
    }

    let n = n.unwrap();

    if let Some(given_name) = n.given_name().map(str::to_string) {
        result.insert("given_name".to_string(), given_name);
    }

    if let Some(generational_suffix) = n.generational_suffix().map(str::to_string) {
        result.insert("generational_suffix".to_string(), generational_suffix);
    }
    if let Some(honorific_prefix) = n.honorific_prefix().map(str::to_string) {
        result.insert("honorific_prefix".to_string(), honorific_prefix);
    }
    if let Some(honorific_suffix) = n.honorific_suffix().map(str::to_string) {
        result.insert("honorific_suffix".to_string(), honorific_suffix);
    }
    if let Some(middle_initials) = n.middle_initials().map(str::to_string) {
        result.insert("middle_initials".to_string(), middle_initials);
    }
    if let Some(middle_name) = n.middle_name() {
        result.insert("middle_name".to_string(), middle_name.to_string());
    }
    result.insert("surname".to_string(), n.surname().to_string());

    Ok(result)
}

/// A Python module implemented in Rust.
#[pymodule]
fn human_name_parser(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_name, m)?)?;
    Ok(())
}

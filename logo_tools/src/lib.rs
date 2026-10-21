//! Numeric kernels of the logos-processing consumer.
//!
//! Each function reproduces the numpy implementation in
//! `ai_center/src/models/logo` bit for bit where float summation order allows,
//! so the Python code stays the reference and the fallback.

mod background;
mod features;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

type FeatureTuple = (f64, f64, f64, f64, f64, f64, f64, f64, f64);

fn check_rgba(rgba: &[u8], width: usize, height: usize) -> PyResult<()> {
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err(PyValueError::new_err(
            "rgba must be width * height * 4 bytes",
        ));
    }
    Ok(())
}

/// Returns (IMAGE_IDF, SKEWNESS, KURTOSIS, HIST_PEAKS, REL_PEAKS, NNS_ARGMAX,
/// TRANSPARENCY_LEVELS, NNS_STD_ERR, OTSU_PARTITION_SCORE) for an RGBA image.
#[pyfunction]
fn image_features(
    py: Python<'_>,
    rgba: &[u8],
    width: usize,
    height: usize,
) -> PyResult<FeatureTuple> {
    check_rgba(rgba, width, height)?;
    let f = py.detach(|| features::image_features(rgba, width, height));
    Ok((
        f.idf,
        f.skewness,
        f.kurtosis,
        f.hist_peaks as f64,
        f.rel_peaks,
        f.nns_argmax as f64,
        f.transparency_levels as f64,
        f.nns_std_err,
        f.otsu_partition_score,
    ))
}

/// True when the logo needs a gray background (many light visible pixels on a
/// partly transparent image); False otherwise.
#[pyfunction]
fn needs_gray_background(
    py: Python<'_>,
    rgba: &[u8],
    width: usize,
    height: usize,
    light_image_threshold: f64,
) -> PyResult<bool> {
    check_rgba(rgba, width, height)?;
    Ok(py.detach(|| background::needs_gray_background(rgba, light_image_threshold)))
}

#[pymodule]
fn logo_tools(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(image_features, m)?)?;
    m.add_function(wrap_pyfunction!(needs_gray_background, m)?)?;
    Ok(())
}

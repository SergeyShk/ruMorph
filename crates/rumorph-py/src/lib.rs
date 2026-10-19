//! Модуль Python `rumorph._rumorph`

use pyo3::prelude::*;

#[pymodule]
fn _rumorph(_m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}

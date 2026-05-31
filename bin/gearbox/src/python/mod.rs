//! Python bindings for gearbox.

use pyo3::prelude::*;
use pyo3::types::PyModule;

#[pyfunction]
fn version() -> &'static str {
    crate::version()
}

pub fn register_python_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

#[pymodule]
fn gearbox(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_python_module(m)
}

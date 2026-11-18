//! Python bindings: `flightlog._flightlog`.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{RecordBatch, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use flightlog_core::{facts, Log as CoreLog, LogKind, ReadOptions};
use pyo3::exceptions::{PyKeyError, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList, PyMemoryView};
use pyo3_arrow::PyTable;

fn to_py_table(py: Python<'_>, batch: RecordBatch) -> PyResult<Py<PyAny>> {
    let schema = batch.schema();
    let t = PyTable::try_new(vec![batch], schema)?;
    Ok(t.into_pyarrow(py)?.unbind())
}

/// A flight log read into memory: one Arrow table per message type.
///
/// `Log(source, types=None, scale=True, keep_order=False)` reads an ArduPilot
/// DataFlash `.bin` or a MAVLink `.tlog` (detected from the content);
/// `source` is a path or the file's bytes.
#[pyclass(name = "Log", module = "flightlog._flightlog", frozen, subclass)]
struct PyLog {
    inner: Arc<CoreLog>,
    path: Option<String>,
}

fn options(types: Option<Vec<String>>, scale: bool, keep_order: bool) -> ReadOptions {
    ReadOptions {
        types: types.map(|t| t.into_iter().collect::<HashSet<_>>()),
        scale,
        keep_order,
    }
}

#[pymethods]
impl PyLog {
    #[new]
    #[pyo3(signature = (source, types=None, scale=true, keep_order=false))]
    fn new(
        py: Python<'_>,
        source: &Bound<'_, PyAny>,
        types: Option<Vec<String>>,
        scale: bool,
        keep_order: bool,
    ) -> PyResult<Self> {
        let opts = options(types, scale, keep_order);
        if source.is_instance_of::<PyBytes>()
            || source.is_instance_of::<PyByteArray>()
            || source.is_instance_of::<PyMemoryView>()
        {
            let data: Vec<u8> = if let Ok(b) = source.cast::<PyBytes>() {
                b.as_bytes().to_vec()
            } else {
                py.import("builtins")?
                    .getattr("bytes")?
                    .call1((source,))?
                    .extract()?
            };
            let log = py
                .detach(|| CoreLog::read(&data, &opts))
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            return Ok(PyLog {
                inner: Arc::new(log),
                path: None,
            });
        }
        let path: std::path::PathBuf = source.extract()?;
        let shown = path.display().to_string();
        let log = py
            .detach(|| CoreLog::open(&path, &opts))
            .map_err(|e| match e {
                flightlog_core::Error::Io(e) => PyOSError::new_err(format!("{shown}: {e}")),
                e => PyValueError::new_err(format!("{shown}: {e}")),
            })?;
        Ok(PyLog {
            inner: Arc::new(log),
            path: Some(shown),
        })
    }

    /// `"dataflash"` or `"tlog"`.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind.name()
    }

    /// File path, when read from a file.
    #[getter]
    fn path(&self) -> Option<String> {
        self.path.clone()
    }

    /// Message types that have a table, in order of first appearance.
    #[getter]
    fn message_types(&self) -> Vec<String> {
        self.inner.names().into_iter().map(String::from).collect()
    }

    /// Messages per type (including types not selected with `types=`).
    #[getter]
    fn counts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (k, v) in &self.inner.counts {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Parameters, last value seen (PARM or PARAM_VALUE).
    #[getter]
    fn params<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (k, v) in &self.inner.params {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// What the reader had to skip: bytes, regions, truncated tail, bad CRCs.
    #[getter]
    fn integrity<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let i = &self.inner.integrity;
        let d = PyDict::new(py);
        d.set_item("bytes", i.bytes)?;
        d.set_item("messages", i.messages)?;
        d.set_item("skipped_bytes", i.skipped_bytes)?;
        d.set_item("skipped_region_count", i.skipped_region_count)?;
        let regions = PyList::empty(py);
        for r in &i.skipped_regions {
            regions.append((r.offset, r.len))?;
        }
        d.set_item("skipped_regions", regions)?;
        d.set_item("truncated_tail_bytes", i.truncated_tail_bytes)?;
        d.set_item("bad_crc", i.bad_crc)?;
        let unknown = PyDict::new(py);
        for (k, v) in &i.unknown_types {
            unknown.set_item(k, v)?;
        }
        d.set_item("unknown_types", unknown)?;
        d.set_item("invalid_formats", i.invalid_formats.clone())?;
        d.set_item("time_outliers", self.inner.time_outliers())?;
        d.set_item("clean", i.is_clean())?;
        Ok(d)
    }

    /// DataFlash: Unix microseconds at TimeUS 0 (from GPS time), else None.
    /// tlog: 0 (timestamps are already Unix microseconds).
    #[getter]
    fn utc_offset_us(&self) -> Option<i64> {
        self.inner.utc_offset_us
    }

    /// `(first, last)` timestamp in the log's time base, or None. Isolated
    /// (damaged) timestamps are left out; `integrity["time_outliers"]`
    /// counts them.
    #[getter]
    fn time_range_us(&self) -> Option<(u64, u64)> {
        self.inner.time_range_us()
    }

    /// Name of the time column: `"TimeUS"` or `"timestamp_us"`.
    #[getter]
    fn time_column(&self) -> &'static str {
        self.inner.time_column()
    }

    /// Field that tells instances apart (`"I"`, `"C"`, `"IMU"`...) for a message, if any.
    fn instance_field(&self, name: &str) -> PyResult<Option<String>> {
        Ok(self.get(name)?.instance_field.clone())
    }

    /// `[(column, arrow type, unit)]` of a message type.
    fn columns(&self, name: &str) -> PyResult<Vec<(String, String, Option<String>)>> {
        let t = self.get(name)?;
        Ok(t.columns
            .iter()
            .map(|c| {
                let ty = match c.data.list_size() {
                    Some(n) => format!("list<{}>[{}]", c.data.data_type().name(), n),
                    None => c.data.data_type().name().to_string(),
                };
                (c.name.clone(), ty, c.unit.clone())
            })
            .collect())
    }

    /// One message type as a `pyarrow.Table`. With `instance`, only rows of
    /// that instance; raises `ValueError` when the message type has no
    /// instance field (see `instance_field()`).
    #[pyo3(signature = (name, instance=None))]
    fn table(&self, py: Python<'_>, name: &str, instance: Option<i64>) -> PyResult<Py<PyAny>> {
        let t = self.get(name)?;
        if instance.is_some() && t.instance_field.is_none() {
            return Err(PyValueError::new_err(format!(
                "{name} has no instance field in this log, so instance= cannot select rows; call table({name:?}) without instance="
            )));
        }
        let batch = py
            .detach(|| match instance {
                Some(i) => t.instance(i).into_record_batch(),
                None => t.to_record_batch(),
            })
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        to_py_table(py, batch)
    }

    /// With `keep_order=True`: a `pyarrow.Table` with `type` (index into
    /// `message_types`) and `row` for every message in file order.
    fn order(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some(o) = &self.inner.order else {
            return Ok(None);
        };
        let t: UInt32Array = o.iter().map(|e| e.0).collect();
        let r: UInt32Array = o.iter().map(|e| e.1).collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("type", DataType::UInt32, false),
            Field::new("row", DataType::UInt32, false),
        ]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(t), Arc::new(r)])
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Some(to_py_table(py, batch)?))
    }

    /// flight-facts v1 as a JSON string.
    #[pyo3(signature = (vibe_percentile=0.95, events=true))]
    fn facts_json(&self, py: Python<'_>, vibe_percentile: f64, events: bool) -> PyResult<String> {
        if !(0.0..=1.0).contains(&vibe_percentile) {
            return Err(PyValueError::new_err(
                "vibe_percentile must be between 0 and 1",
            ));
        }
        let log = Arc::clone(&self.inner);
        Ok(py.detach(move || {
            let opts = facts::Options {
                vibe_percentile,
                events,
                ..facts::Options::default()
            };
            facts::run(&log, &opts).to_json()
        }))
    }

    fn __len__(&self) -> usize {
        self.inner.integrity.messages as usize
    }

    fn __contains__(&self, name: &str) -> bool {
        self.inner.table(name).is_some()
    }

    fn __repr__(&self) -> String {
        format!(
            "<flightlog.Log {} {} messages, {} types{}>",
            self.inner.kind.name(),
            self.inner.integrity.messages,
            self.inner.tables().len(),
            self.path
                .as_ref()
                .map(|p| format!(" from {p}"))
                .unwrap_or_default()
        )
    }
}

impl PyLog {
    fn get(&self, name: &str) -> PyResult<&flightlog_core::Table> {
        self.inner
            .table(name)
            .ok_or_else(|| PyKeyError::new_err(format!("no {name} messages in this log")))
    }
}

/// `"dataflash"`, `"tlog"` or None, from the first bytes of a file.
#[pyfunction]
fn detect(data: &[u8]) -> Option<&'static str> {
    LogKind::detect(data).map(|k| k.name())
}

/// The flight-facts v1 JSON Schema as a string.
#[pyfunction]
fn facts_schema() -> &'static str {
    facts::SCHEMA
}

#[pymodule]
fn _flightlog(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyLog>()?;
    m.add_function(wrap_pyfunction!(detect, m)?)?;
    m.add_function(wrap_pyfunction!(facts_schema, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("FACTS_VERSION", facts::VERSION)?;
    Ok(())
}

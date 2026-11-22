//! Compares the ULog reader with pyulog 1.3.0 on every log that has a
//! `<log>.pyulog.json` in `testdata/` (see tools/pyulog_reference.py). The
//! logs themselves are pyulog's test files, downloaded to
//! `testdata/external/ulog/` by tools/fetch_testdata.sh; missing logs are
//! skipped.

use std::path::{Path, PathBuf};

use flightlog_core::ulog::ULog;
use flightlog_core::{ColumnData, FlightLog, ReadOptions};
use serde_json::Value;

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
}

fn references() -> Vec<(PathBuf, Value)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(testdata()).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(log) = name.strip_suffix(".pyulog.json") {
            let path = testdata().join("external").join("ulog").join(log);
            let v: Value = serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap();
            out.push((path, v));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn sum(c: &ColumnData) -> Value {
    fn ints<T: Copy + Into<i128>>(v: &[T]) -> Value {
        Value::String(v.iter().map(|&x| x.into()).sum::<i128>().to_string())
    }
    match c {
        ColumnData::Int8(v) => ints(v),
        ColumnData::UInt8(v) => ints(v),
        ColumnData::Int16(v) => ints(v),
        ColumnData::UInt16(v) => ints(v),
        ColumnData::Int32(v) => ints(v),
        ColumnData::UInt32(v) => ints(v),
        ColumnData::Int64(v) => ints(v),
        ColumnData::UInt64(v) => ints(v),
        ColumnData::Float32(v) => Value::String(format!(
            "{:?}",
            v.iter()
                .filter(|x| !x.is_nan())
                .fold(0.0f64, |a, &x| a + x as f64)
        )),
        ColumnData::Float64(v) => Value::String(format!(
            "{:?}",
            v.iter().filter(|x| !x.is_nan()).fold(0.0f64, |a, &x| a + x)
        )),
        _ => panic!("ULog columns are numeric"),
    }
}

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
        // summation order differs (pyulog sums per instance, pairwise)
        || (a.is_infinite() && b.is_infinite())
}

#[test]
fn ulog_matches_pyulog() {
    let refs = references();
    assert!(!refs.is_empty(), "no pyulog references in testdata/");
    let mut checked = 0;
    for (path, r) in refs {
        if !path.exists() {
            println!("SKIP {} (run tools/fetch_testdata.sh)", path.display());
            continue;
        }
        let log = ULog::open(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let topics = r["topics"].as_object().unwrap();
        assert_eq!(
            log.tables().len(),
            topics.len(),
            "{name}: topics {:?}",
            log.names()
        );
        for (topic, t) in topics {
            let table = log
                .table(topic)
                .unwrap_or_else(|| panic!("{name}: no {topic}"));
            assert_eq!(
                table.len() as u64,
                t["count"].as_u64().unwrap(),
                "{name} {topic} rows"
            );
            let fields = t["fields"].as_object().unwrap();
            // our tables add multi_id
            assert_eq!(
                table.columns.len(),
                fields.len() + 1,
                "{name} {topic} columns"
            );
            for (field, want) in fields {
                let col = table
                    .column(field)
                    .unwrap_or_else(|| panic!("{name}: no {topic}.{field}"));
                let got = sum(&col.data);
                let (Value::String(w), Value::String(g)) = (want, &got) else {
                    panic!("{name} {topic}.{field}: bad reference {want}");
                };
                if matches!(col.data, ColumnData::Float32(_) | ColumnData::Float64(_)) {
                    let w: f64 = w.parse().unwrap_or(f64::NAN);
                    let g: f64 = g.parse().unwrap();
                    assert!(
                        close(w, g),
                        "{name} {topic}.{field}: pyulog {w} flightlog {g}"
                    );
                } else {
                    assert_eq!(w, g, "{name} {topic}.{field}");
                }
            }
        }
        let params = r["params"].as_object().unwrap();
        assert_eq!(log.params.len(), params.len(), "{name} params");
        for (k, v) in params {
            let want = v.as_f64().unwrap();
            let got = log
                .param(k)
                .unwrap_or_else(|| panic!("{name}: no param {k}"));
            // pyulog keeps the f32 bits, flightlog the shortest decimal
            assert_eq!(got as f32, want as f32, "{name} param {k}");
        }
        assert_eq!(
            log.changed_params.len() as u64,
            r["changed_params"].as_u64().unwrap(),
            "{name} changed params"
        );
        assert_eq!(
            log.logged.len() as u64,
            r["logged"].as_u64().unwrap(),
            "{name} logged"
        );
        assert_eq!(
            log.dropouts.len() as u64,
            r["dropouts"].as_u64().unwrap(),
            "{name} dropouts"
        );
        let keys: Vec<&str> = log.info.keys().map(String::as_str).collect();
        let want: Vec<&str> = r["info_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(keys, want, "{name} info keys");
        // a clean file: every byte used
        if !r["file_corrupt"].as_bool().unwrap() && log.appended_offsets.is_empty() {
            assert_eq!(
                log.integrity.skipped_bytes, 0,
                "{name}: {:?}",
                log.integrity
            );
        }
        println!(
            "{name}: {} topics, {} messages equal to pyulog",
            topics.len(),
            log.integrity.messages
        );
        checked += 1;
    }
    println!("{checked} ULog files checked");
}

fn sample() -> Option<Vec<u8>> {
    let p = testdata().join("external").join("ulog").join("sample.ulg");
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            println!("SKIP {} (run tools/fetch_testdata.sh)", p.display());
            None
        }
    }
}

#[test]
fn damaged_ulog_still_opens() {
    let Some(bytes) = sample() else { return };
    let clean = ULog::read(&bytes).unwrap();
    let rows = |l: &ULog| l.tables().iter().map(|t| t.len()).sum::<usize>();
    let total = rows(&clean);

    // 64 bytes overwritten in the middle of the data
    let mut b = bytes.clone();
    let mid = b.len() / 2;
    for x in &mut b[mid..mid + 64] {
        *x = !*x;
    }
    let l = ULog::read(&b).unwrap();
    assert!(!l.integrity.is_clean());
    assert!(l.integrity.skipped_bytes > 0);
    assert!(rows(&l) > total * 9 / 10, "{} of {total}", rows(&l));

    // cut in half
    let l = ULog::read(&bytes[..bytes.len() / 2]).unwrap();
    assert!(rows(&l) > total / 3 && rows(&l) < total);

    // garbage inserted
    let mut b = bytes[..mid].to_vec();
    b.extend(std::iter::repeat_n(0u8, 5000));
    b.extend_from_slice(&bytes[mid..]);
    let l = ULog::read(&b).unwrap();
    // the message cut at the insertion point may take some of the zeros
    assert!(l.integrity.skipped_bytes >= 4000, "{:?}", l.integrity);
    assert!(rows(&l) > total * 9 / 10);
}

#[test]
fn flight_log_opens_every_format() {
    let p = testdata()
        .join("external")
        .join("ulog")
        .join("sample_logging_tagged_and_default_params.ulg");
    let Ok(bytes) = std::fs::read(&p) else {
        println!("SKIP {}", p.display());
        return;
    };
    let log = FlightLog::read(&bytes, &ReadOptions::default()).unwrap();
    assert_eq!(log.format_name(), "ulog");
    assert_eq!(log.time_column(), "timestamp");
    let (a, b) = log.time_range_us().unwrap();
    assert!(b > a);
    let track = log.track();
    assert!(!track.is_empty());
    for p in &track {
        assert!(p.lat.abs() <= 90.0 && p.lon.abs() <= 180.0);
    }
    let gpx = flightlog_core::track::to_gpx(&track, "sample");
    assert_eq!(gpx.matches("<trkpt").count(), track.len());
    let ev = flightlog_core::events::from_ulog(log.as_px4().unwrap());
    assert!(ev
        .iter()
        .any(|e| e.kind == flightlog_core::events::EventKind::Mode));
    for w in ev.windows(2) {
        assert!(w[0].time_us <= w[1].time_us);
    }
}

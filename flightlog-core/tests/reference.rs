//! Compares flightlog-core with pymavlink on every log that has a
//! `<log>.pymavlink.json` next to it in `testdata/` or `testdata/external/`
//! (see tools/pymavlink_reference.py). Logs not downloaded are skipped.

use std::path::{Path, PathBuf};

use flightlog_core::{ColumnData, Log, ReadOptions, Table};
use serde_json::Value;

fn crc32(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
}

fn references() -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for dir in [testdata(), testdata().join("external")] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if let Some(log) = name.strip_suffix(".pymavlink.json") {
                let local = dir.join(log);
                let path = if local.exists() {
                    local
                } else {
                    testdata().join("external").join(log)
                };
                out.push((path, p));
            }
        }
    }
    out.sort();
    out
}

enum Fp {
    Int(i128),
    Float(f64),
    Crc(&'static str, u32),
    Empty,
}

fn fingerprint(c: &ColumnData) -> Fp {
    fn ints<T: Copy + Into<i128>>(v: &[T]) -> Fp {
        Fp::Int(v.iter().map(|&x| x.into()).sum())
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
        ColumnData::Float32(v) => Fp::Float(v.iter().fold(0.0f64, |a, &x| a + x as f64)),
        ColumnData::Float64(v) => Fp::Float(v.iter().fold(0.0f64, |a, &x| a + x)),
        ColumnData::Utf8(v) | ColumnData::Binary(v) => {
            let mut crc = 0;
            for i in 0..v.len() {
                crc = crc32(crc, v.get(i));
                crc = crc32(crc, &[0]);
            }
            Fp::Crc(
                if matches!(c, ColumnData::Utf8(_)) {
                    "text"
                } else {
                    "bytes"
                },
                crc,
            )
        }
        ColumnData::FixedList { values, .. } => {
            if values.is_empty() {
                Fp::Empty
            } else {
                fingerprint(values)
            }
        }
    }
}

fn same_float(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

fn parse_float(s: &str) -> f64 {
    match s {
        "nan" => f64::NAN,
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        s => s.parse().unwrap(),
    }
}

fn check_table(
    log_name: &str,
    t: &Table,
    r: &Value,
    errors: &mut Vec<String>,
    recovered: &mut u64,
) {
    let count = r["count"].as_u64().unwrap() as usize;
    if t.len() > count && log_name.ends_with(".tlog") {
        // pymavlink loses some valid (checksum-verified) frames next to
        // non-MAVLink bytes in a tlog; we keep them. Values are compared on
        // the *-clean.tlog copy instead.
        println!(
            "  {log_name} {}: {} rows, pymavlink {} (recovered {})",
            t.name,
            t.len(),
            count,
            t.len() - count
        );
        *recovered += (t.len() - count) as u64;
        return;
    }
    if t.len() != count {
        errors.push(format!(
            "{log_name} {}: {} rows, pymavlink {}",
            t.name,
            t.len(),
            count
        ));
        return;
    }
    let fields = r["fields"].as_object().unwrap();
    for (fname, expect) in fields {
        if fname == "timestamp_us" {
            continue;
        }
        let Some(col) = t.column(fname) else {
            errors.push(format!("{log_name} {}: missing column {fname}", t.name));
            continue;
        };
        let fp = fingerprint(&col.data);
        let ok = match (&fp, expect.as_object().and_then(|o| o.iter().next())) {
            (Fp::Int(v), Some((k, e))) if k == "int" => {
                e.as_str().unwrap().parse::<i128>().unwrap() == *v
            }
            (Fp::Float(v), Some((k, e))) if k == "float" => {
                same_float(parse_float(e.as_str().unwrap()), *v)
            }
            // pymavlink gives ints for float columns that only hold whole numbers? no: compare as float.
            (Fp::Int(v), Some((k, e))) if k == "float" => {
                same_float(parse_float(e.as_str().unwrap()), *v as f64)
            }
            (Fp::Crc(kind, v), Some((k, e))) if k == kind => e.as_u64().unwrap() as u32 == *v,
            (Fp::Empty, None) => true,
            (_, None) => count == 0,
            _ => false,
        };
        if !ok {
            let got = match fp {
                Fp::Int(v) => format!("int {v}"),
                Fp::Float(v) => format!("float {v:?}"),
                Fp::Crc(k, v) => format!("{k} {v}"),
                Fp::Empty => "empty".into(),
            };
            errors.push(format!(
                "{log_name} {}.{fname}: got {got}, pymavlink {expect}",
                t.name
            ));
        }
    }
}

#[test]
fn matches_pymavlink() {
    let refs = references();
    let mut checked = 0;
    let mut errors = Vec::new();
    let mut recovered = 0u64;
    for (log_path, ref_path) in refs {
        let name = log_path.file_name().unwrap().to_string_lossy().to_string();
        if !log_path.exists() {
            println!("SKIP {name}: not downloaded (tools/fetch_testdata.sh)");
            continue;
        }
        let r: Value = serde_json::from_slice(&std::fs::read(&ref_path).unwrap()).unwrap();
        let log = Log::open(&log_path, &ReadOptions::default()).unwrap();
        let types = r["types"].as_object().unwrap();
        for (tname, tref) in types {
            match log.table(tname) {
                Some(t) => check_table(&name, t, tref, &mut errors, &mut recovered),
                None => errors.push(format!("{name}: no table {tname}")),
            }
        }
        for t in log.tables() {
            if !types.contains_key(&t.name) {
                errors.push(format!(
                    "{name}: table {} ({} rows) not in pymavlink",
                    t.name,
                    t.len()
                ));
            }
        }
        checked += 1;
        println!("ok {name}: {} types", types.len());
    }
    assert!(
        errors.is_empty(),
        "{} differences:\n{}",
        errors.len(),
        errors.join("\n")
    );
    println!("{checked} logs checked, {recovered} tlog frames pymavlink did not return");
}

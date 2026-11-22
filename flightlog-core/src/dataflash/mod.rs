//! ArduPilot DataFlash (`.bin`) reader.
//!
//! A DataFlash log is a stream of messages, each `0xA3 0x95 <type>` followed
//! by a fixed-length body. FMT messages (type 128) define the other types:
//! name, length, a format string and field labels. FMTU, UNIT and MULT add
//! units and mark the instance field.
//!
//! Recovery follows pymavlink's `DFReader_binary._parse_next`: a message
//! starts at a header whose type has been defined; anything else is skipped
//! one byte at a time; a message cut off by the end of the file is dropped.
//! The skipped runs are reported in [`crate::Integrity`].

mod format;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub use format::{f16_to_f32, field_size, Format};

use crate::column::{Column, ColumnData, Table, VarData};
use crate::log::{Integrity, Log, LogKind, ReadOptions};

/// First header byte.
pub const HEAD1: u8 = 0xA3;
/// Second header byte.
pub const HEAD2: u8 = 0x95;
/// Type id of FMT.
pub const FMT_TYPE: u8 = 0x80;

struct Builder {
    name: String,
    fmt: Arc<Format>,
    columns: Vec<ColumnData>,
    rows: usize,
}

struct TypeState {
    fmt: Arc<Format>,
    builder: Option<usize>,
}

/// Reads a whole DataFlash log from memory.
pub fn read(bytes: &[u8], opts: &ReadOptions) -> Log {
    let mut types: Vec<Option<TypeState>> = (0..256).map(|_| None).collect();
    types[FMT_TYPE as usize] = Some(TypeState {
        fmt: Arc::new(Format::bootstrap_fmt()),
        builder: None,
    });
    let mut builders: Vec<Builder> = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut params: BTreeMap<String, f64> = BTreeMap::new();
    let mut integrity = Integrity {
        bytes: bytes.len() as u64,
        ..Integrity::default()
    };
    let mut fmtu: HashMap<u8, (String, String)> = HashMap::new();
    let mut unit_lookup: HashMap<u8, String> = HashMap::new();
    let mut mult_lookup: HashMap<u8, f64> = HashMap::new();
    let mut utc_offset_us: Option<i64> = None;
    let mut time_search_done = false;
    let mut order: Option<Vec<(u32, u32)>> = opts.keep_order.then(Vec::new);

    let n = bytes.len();
    let mut ofs = 0usize;
    let mut skip_start: Option<usize> = None;

    while n - ofs >= 3 {
        let t = bytes[ofs + 2];
        let is_msg = bytes[ofs] == HEAD1 && bytes[ofs + 1] == HEAD2 && types[t as usize].is_some();
        if !is_msg {
            skip_start.get_or_insert(ofs);
            ofs += 1;
            continue;
        }
        if let Some(s) = skip_start.take() {
            integrity.skip(s as u64, (ofs - s) as u64);
        }
        let state = types[t as usize].as_ref().expect("checked");
        let fmt = Arc::clone(&state.fmt);
        if !fmt.is_usable() {
            // pymavlink fails to unpack these and resumes right after the header.
            *integrity.unknown_types.entry(t as u32).or_insert(0) += 1;
            integrity.skip(ofs as u64, 3);
            ofs += 3;
            continue;
        }
        let body_len = fmt.len - 3;
        if n - ofs - 3 < body_len {
            integrity.truncated_tail_bytes = (n - ofs) as u64;
            ofs = n;
            break;
        }
        let body = &bytes[ofs + 3..ofs + 3 + body_len];

        if t == FMT_TYPE {
            match Format::from_fmt_body(body) {
                Ok(new_fmt) => {
                    let id = new_fmt.type_id as usize;
                    let new_fmt = Arc::new(new_fmt);
                    match &mut types[id] {
                        Some(old) if old.fmt.same_layout(&new_fmt) => {}
                        Some(old) => {
                            if old.builder.is_some_and(|b| builders[b].rows > 0) {
                                integrity.invalid_formats.push(format!(
                                    "type {id} redefined from {} to {} after use",
                                    old.fmt.name, new_fmt.name
                                ));
                            }
                            old.fmt = new_fmt;
                            old.builder = None;
                        }
                        slot @ None => {
                            *slot = Some(TypeState {
                                fmt: new_fmt,
                                builder: None,
                            })
                        }
                    }
                }
                Err(e) => {
                    // pymavlink drops the FMT message and resumes after its header.
                    integrity.invalid_formats.push(e);
                    integrity.skip(ofs as u64, 3);
                    ofs += 3;
                    continue;
                }
            }
        }

        let name = fmt.name.as_str();
        match name {
            "FMTU" => {
                if let (Some(ft), Some(u), m) = (
                    fmt.num(body, "FmtType"),
                    fmt.text(body, "UnitIds"),
                    fmt.text(body, "MultIds"),
                ) {
                    fmtu.insert(ft as u8, (u, m.unwrap_or_default()));
                }
            }
            "UNIT" => {
                if let (Some(id), Some(label)) = (fmt.num(body, "Id"), fmt.text(body, "Label")) {
                    unit_lookup.insert(id as u8, label);
                }
            }
            "MULT" => {
                if let (Some(id), Some(m)) = (fmt.num(body, "Id"), fmt.num(body, "Mult")) {
                    // Same rounding as pymavlink: the double holds a float value.
                    mult_lookup.insert(id as u8, round_sig7(m));
                }
            }
            "PARM" => {
                if let (Some(pn), Some(v)) = (fmt.text(body, "Name"), fmt.num(body, "Value")) {
                    params.insert(
                        pn,
                        if fmt.is_f32("Value") {
                            crate::short_f32(v as f32)
                        } else {
                            v
                        },
                    );
                }
            }
            "GPS" if !time_search_done => {
                match (
                    fmt.num(body, "TimeUS"),
                    fmt.num(body, "GWk"),
                    fmt.num(body, "GMS"),
                ) {
                    (Some(tus), Some(wk), Some(ms)) => {
                        if wk > 0.0 {
                            utc_offset_us =
                                Some(gps_utc_offset_us(wk as i64, ms as i64, tus as i64));
                            time_search_done = true;
                        }
                    }
                    _ => time_search_done = true,
                }
            }
            _ => {}
        }

        *counts.entry(fmt.name.clone()).or_insert(0) += 1;
        integrity.messages += 1;

        if opts.wants(name) {
            let state = types[t as usize].as_mut().expect("checked");
            // FMT may just have redefined its own type id; rows always go to
            // the builder of the format the body was decoded with.
            let b = match state.builder {
                Some(b) if Arc::ptr_eq(&builders[b].fmt, &fmt) => b,
                _ => {
                    builders.push(Builder {
                        name: fmt.name.clone(),
                        columns: fmt.empty_columns(opts.scale),
                        fmt: Arc::clone(&fmt),
                        rows: 0,
                    });
                    let b = builders.len() - 1;
                    if Arc::ptr_eq(&state.fmt, &fmt) {
                        state.builder = Some(b);
                    }
                    b
                }
            };
            let builder = &mut builders[b];
            fmt.decode_into(body, &mut builder.columns, opts.scale);
            if let Some(o) = order.as_mut() {
                o.push((b as u32, builder.rows as u32));
            }
            builder.rows += 1;
        }

        ofs += fmt.len;
    }
    if let Some(s) = skip_start.take() {
        integrity.skip(s as u64, (ofs - s) as u64);
    }
    if ofs < n {
        // Fewer than 3 bytes left: not a message.
        integrity.truncated_tail_bytes += (n - ofs) as u64;
    }

    // Tables, with units resolved from the final UNIT/MULT/FMTU lookups.
    let mut tables = Vec::with_capacity(builders.len());
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut remap: Vec<u32> = Vec::with_capacity(builders.len());
    let mut base: Vec<u32> = Vec::with_capacity(builders.len());
    for b in builders {
        let units = fmtu
            .get(&b.fmt.type_id)
            .map(|(u, m)| b.fmt.units(u, m, &unit_lookup, &mult_lookup));
        let instance_field = fmtu
            .get(&b.fmt.type_id)
            .and_then(|(u, _)| u.find('#'))
            .and_then(|i| b.fmt.labels.get(i).cloned());
        let mut columns = Vec::with_capacity(b.columns.len());
        for (i, mut data) in b.columns.into_iter().enumerate() {
            data.shrink();
            let (unit, multiplier) = units
                .as_ref()
                .and_then(|u| u.get(i).cloned())
                .unwrap_or((None, None));
            columns.push(Column {
                name: b.fmt.labels[i].clone(),
                unit,
                multiplier,
                data,
            });
        }
        let table = Table {
            name: b.name.clone(),
            columns,
            instance_field,
            format: Some(b.fmt.format.clone()),
            rows: b.rows,
        };
        // Same name and layout as an earlier table: append rows to it.
        if let Some(&prev) = taken.get(&b.name) {
            let prev_t: &mut Table = &mut tables[prev];
            if prev_t.format == table.format && prev_t.column_names() == table.column_names() {
                remap.push(prev as u32);
                base.push(prev_t.len() as u32);
                append(prev_t, table);
                continue;
            }
            let mut k = 2;
            let mut nm = format!("{}#{}", b.name, k);
            while taken.contains_key(&nm) {
                k += 1;
                nm = format!("{}#{}", b.name, k);
            }
            let mut table = table;
            table.name = nm.clone();
            taken.insert(nm, tables.len());
            remap.push(tables.len() as u32);
            base.push(0);
            tables.push(table);
            continue;
        }
        taken.insert(b.name.clone(), tables.len());
        remap.push(tables.len() as u32);
        base.push(0);
        tables.push(table);
    }
    if let Some(o) = order.as_mut() {
        fix_order(o, &remap, &base);
    }

    Log::from_parts(
        LogKind::DataFlash,
        tables,
        counts,
        params,
        integrity,
        utc_offset_us,
        order,
    )
}

/// Points order entries at the final tables; rows of a builder merged into
/// an earlier table come after that table's earlier rows.
fn fix_order(order: &mut [(u32, u32)], remap: &[u32], base: &[u32]) {
    for e in order.iter_mut() {
        let b = e.0 as usize;
        e.0 = remap[b];
        e.1 += base[b];
    }
}

fn append(dst: &mut Table, src: Table) {
    dst.rows += src.rows;
    for (d, s) in dst.columns.iter_mut().zip(src.columns) {
        append_data(&mut d.data, s.data);
    }
}

fn append_data(d: &mut ColumnData, s: ColumnData) {
    match (d, s) {
        (ColumnData::Int8(a), ColumnData::Int8(b)) => a.extend(b),
        (ColumnData::UInt8(a), ColumnData::UInt8(b)) => a.extend(b),
        (ColumnData::Int16(a), ColumnData::Int16(b)) => a.extend(b),
        (ColumnData::UInt16(a), ColumnData::UInt16(b)) => a.extend(b),
        (ColumnData::Int32(a), ColumnData::Int32(b)) => a.extend(b),
        (ColumnData::UInt32(a), ColumnData::UInt32(b)) => a.extend(b),
        (ColumnData::Int64(a), ColumnData::Int64(b)) => a.extend(b),
        (ColumnData::UInt64(a), ColumnData::UInt64(b)) => a.extend(b),
        (ColumnData::Float32(a), ColumnData::Float32(b)) => a.extend(b),
        (ColumnData::Float64(a), ColumnData::Float64(b)) => a.extend(b),
        (ColumnData::Utf8(a), ColumnData::Utf8(b))
        | (ColumnData::Binary(a), ColumnData::Binary(b)) => append_var(a, &b),
        (ColumnData::FixedList { values: a, .. }, ColumnData::FixedList { values: b, .. }) => {
            append_data(a, *b)
        }
        _ => unreachable!("same format string gives same column types"),
    }
}

fn append_var(a: &mut VarData, b: &VarData) {
    for i in 0..b.len() {
        a.push(b.get(i));
    }
}

/// Unix microseconds at `TimeUS == 0`, from a GPS week, ms of week and the
/// message's TimeUS. GPS epoch 1980-01-06, minus 18 leap seconds, as pymavlink.
pub fn gps_utc_offset_us(week: i64, ms: i64, time_us: i64) -> i64 {
    const GPS_EPOCH_UNIX_S: i64 = 315_964_800;
    (GPS_EPOCH_UNIX_S + week * 604_800 - 18) * 1_000_000 + ms * 1_000 - time_us
}

/// Rounds to 7 significant digits (pymavlink `float("%.7g" % x)`).
fn round_sig7(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{:.6e}", x).parse().unwrap_or(x)
}

#[doc(hidden)]
pub fn decode_text_pub(b: &[u8]) -> String {
    format::decode_text(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gps_epoch() {
        // Week 0, ms 0, TimeUS 0 -> 1980-01-06 minus the 18 s leap offset.
        assert_eq!(gps_utc_offset_us(0, 0, 0), (315_964_800 - 18) * 1_000_000);
    }

    #[test]
    fn sig7() {
        assert_eq!(round_sig7(0.009999999776482582), 0.01);
        assert_eq!(round_sig7(1e-7), 1e-7);
    }
}

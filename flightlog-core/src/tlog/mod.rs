//! MAVLink `.tlog` reader: frames from [`mavlink_tlog`], fields decoded with
//! the `all.xml` layouts in [`defs`].
//!
//! Each message type becomes a table with `timestamp_us` (the logger's Unix
//! microseconds), `sys_id`, `comp_id`, then the message fields in XML order.
//! Frames are padded to the full payload length before decoding, so
//! MAVLink 2 trimmed payloads and MAVLink 1 frames without extension fields
//! read as zeros, as in pymavlink.

#[rustfmt::skip]
pub mod defs;

use std::collections::{BTreeMap, HashMap};

use mavlink_tlog::TlogSlice;

use crate::column::{Column, ColumnData, DataType, Table};
use crate::log::{Integrity, Log, LogKind, ReadOptions};

/// Element type of a MAVLink field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    /// `uint8_t`
    U8,
    /// `int8_t`
    I8,
    /// `uint16_t`
    U16,
    /// `int16_t`
    I16,
    /// `uint32_t`
    U32,
    /// `int32_t`
    I32,
    /// `uint64_t`
    U64,
    /// `int64_t`
    I64,
    /// `float`
    F32,
    /// `double`
    F64,
    /// `char`
    Char,
}

impl Ty {
    /// Size in bytes.
    pub fn size(self) -> usize {
        match self {
            Ty::U8 | Ty::I8 | Ty::Char => 1,
            Ty::U16 | Ty::I16 => 2,
            Ty::U32 | Ty::I32 | Ty::F32 => 4,
            Ty::U64 | Ty::I64 | Ty::F64 => 8,
        }
    }

    fn data_type(self) -> DataType {
        match self {
            Ty::U8 => DataType::UInt8,
            Ty::I8 => DataType::Int8,
            Ty::U16 => DataType::UInt16,
            Ty::I16 => DataType::Int16,
            Ty::U32 => DataType::UInt32,
            Ty::I32 => DataType::Int32,
            Ty::U64 => DataType::UInt64,
            Ty::I64 => DataType::Int64,
            Ty::F32 => DataType::Float32,
            Ty::F64 => DataType::Float64,
            Ty::Char => DataType::Utf8,
        }
    }
}

/// One field of a message.
#[derive(Debug, Clone, Copy)]
pub struct FieldDef {
    /// Field name.
    pub name: &'static str,
    /// Element type.
    pub ty: Ty,
    /// Array length, 0 for a scalar.
    pub array: usize,
    /// Byte offset in the payload.
    pub offset: usize,
    /// Unit from the XML, empty when none.
    pub unit: &'static str,
}

/// One message definition.
#[derive(Debug, Clone, Copy)]
pub struct MessageDef {
    /// Message id.
    pub id: u32,
    /// Message name.
    pub name: &'static str,
    /// Full payload length (with extensions).
    pub len: usize,
    /// Fields in XML order.
    pub fields: &'static [FieldDef],
}

/// The definition of a message id.
pub fn message_def(id: u32) -> Option<&'static MessageDef> {
    defs::MESSAGES
        .binary_search_by_key(&id, |m| m.id)
        .ok()
        .map(|i| &defs::MESSAGES[i])
}

/// The definition of a message name.
pub fn message_def_by_name(name: &str) -> Option<&'static MessageDef> {
    defs::MESSAGES.iter().find(|m| m.name == name)
}

struct Builder {
    def: &'static MessageDef,
    ts: Vec<u64>,
    sys: Vec<u8>,
    comp: Vec<u8>,
    cols: Vec<ColumnData>,
}

impl Builder {
    fn new(def: &'static MessageDef) -> Self {
        Builder {
            def,
            ts: Vec::new(),
            sys: Vec::new(),
            comp: Vec::new(),
            cols: def
                .fields
                .iter()
                .map(|f| {
                    if f.ty == Ty::Char {
                        ColumnData::empty(DataType::Utf8, 0)
                    } else {
                        ColumnData::empty(f.ty.data_type(), f.array)
                    }
                })
                .collect(),
        }
    }

    fn push(&mut self, payload: &[u8]) {
        for (f, col) in self.def.fields.iter().zip(self.cols.iter_mut()) {
            let n = f.array.max(1);
            let b = &payload[f.offset..f.offset + f.ty.size() * n];
            push_field(col, f.ty, b);
        }
    }
}

fn push_field(col: &mut ColumnData, ty: Ty, b: &[u8]) {
    macro_rules! ext {
        ($v:expr, $t:ty, $n:expr) => {
            $v.extend(
                b.chunks_exact($n)
                    .map(|c| <$t>::from_le_bytes(c.try_into().expect("size"))),
            )
        };
    }
    let target = match col {
        ColumnData::FixedList { values, .. } => values.as_mut(),
        other => other,
    };
    match (target, ty) {
        (ColumnData::UInt8(v), _) => v.extend_from_slice(b),
        (ColumnData::Int8(v), _) => v.extend(b.iter().map(|&x| x as i8)),
        (ColumnData::UInt16(v), _) => ext!(v, u16, 2),
        (ColumnData::Int16(v), _) => ext!(v, i16, 2),
        (ColumnData::UInt32(v), _) => ext!(v, u32, 4),
        (ColumnData::Int32(v), _) => ext!(v, i32, 4),
        (ColumnData::UInt64(v), _) => ext!(v, u64, 8),
        (ColumnData::Int64(v), _) => ext!(v, i64, 8),
        (ColumnData::Float32(v), _) => ext!(v, f32, 4),
        (ColumnData::Float64(v), _) => ext!(v, f64, 8),
        (ColumnData::Utf8(v), Ty::Char) => {
            // pymavlink: char arrays are text up to the first NUL.
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            v.push(crate::dataflash_text(&b[..end]).as_bytes());
        }
        _ => unreachable!("column built from the same definition"),
    }
}

/// Reads a whole tlog from memory.
pub fn read(bytes: &[u8], opts: &ReadOptions) -> Log {
    let mut slice = TlogSlice::new(bytes);
    let mut builders: Vec<Builder> = Vec::new();
    let mut by_id: HashMap<u32, usize> = HashMap::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut params: BTreeMap<String, f64> = BTreeMap::new();
    let mut integrity = Integrity {
        bytes: bytes.len() as u64,
        ..Integrity::default()
    };
    let mut order: Option<Vec<(u32, u32)>> = opts.keep_order.then(Vec::new);
    let mut padded = [0u8; 255];
    let mut last_pos = 0usize;

    while let Some(rec) = slice.next() {
        // Skipped bytes show up as a gap between records.
        let start = slice.position() - (mavlink_tlog::TIMESTAMP_LEN + rec.frame.len());
        if start > last_pos {
            integrity.skip(last_pos as u64, (start - last_pos) as u64);
        }
        last_pos = slice.position();

        let id = rec.frame.msg_id();
        let Some(def) = message_def(id) else {
            *integrity.unknown_types.entry(id).or_insert(0) += 1;
            continue;
        };
        integrity.messages += 1;
        *counts.entry(def.name.to_string()).or_insert(0) += 1;
        rec.frame.payload_padded(&mut padded);
        let payload = &padded[..];

        if id == 22 {
            // PARAM_VALUE: param_id char[16], param_value float
            let pid = def.fields.iter().find(|f| f.name == "param_id");
            let pv = def.fields.iter().find(|f| f.name == "param_value");
            if let (Some(pid), Some(pv)) = (pid, pv) {
                let raw = &payload[pid.offset..pid.offset + 16];
                let end = raw.iter().position(|&c| c == 0).unwrap_or(16);
                let name = crate::dataflash_text(&raw[..end]);
                let v =
                    f32::from_le_bytes(payload[pv.offset..pv.offset + 4].try_into().expect("4"));
                params.insert(name, crate::short_f32(v));
            }
        }

        if !opts.wants(def.name) {
            continue;
        }
        let b = *by_id.entry(id).or_insert_with(|| {
            builders.push(Builder::new(def));
            builders.len() - 1
        });
        let builder = &mut builders[b];
        if let Some(o) = order.as_mut() {
            o.push((b as u32, builder.ts.len() as u32));
        }
        builder.ts.push(rec.timestamp_us);
        builder.sys.push(rec.frame.sys_id());
        builder.comp.push(rec.frame.comp_id());
        builder.push(payload);
    }
    let st = slice.stats();
    if bytes.len() as u64 - st.truncated_tail > last_pos as u64 {
        integrity.skip(
            last_pos as u64,
            bytes.len() as u64 - st.truncated_tail - last_pos as u64,
        );
    }
    integrity.truncated_tail_bytes = st.truncated_tail;
    integrity.bad_crc = st.bad_crc;

    let tables = builders
        .into_iter()
        .map(|b| {
            let rows = b.ts.len();
            let mut columns = vec![
                Column {
                    name: "timestamp_us".into(),
                    unit: Some("us".into()),
                    multiplier: None,
                    data: ColumnData::UInt64(b.ts),
                },
                Column {
                    name: "sys_id".into(),
                    unit: None,
                    multiplier: None,
                    data: ColumnData::UInt8(b.sys),
                },
                Column {
                    name: "comp_id".into(),
                    unit: None,
                    multiplier: None,
                    data: ColumnData::UInt8(b.comp),
                },
            ];
            for (f, mut data) in b.def.fields.iter().zip(b.cols) {
                data.shrink();
                columns.push(Column {
                    name: f.name.to_string(),
                    unit: (!f.unit.is_empty()).then(|| f.unit.to_string()),
                    multiplier: None,
                    data,
                });
            }
            Table {
                name: b.def.name.to_string(),
                columns,
                instance_field: None,
                format: None,
                rows,
            }
        })
        .collect();

    Log::from_parts(
        LogKind::Tlog,
        tables,
        counts,
        params,
        integrity,
        Some(0),
        order,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defs_sorted_and_consistent() {
        for w in defs::MESSAGES.windows(2) {
            assert!(w[0].id < w[1].id);
        }
        for m in defs::MESSAGES {
            assert!(m.len <= 255, "{}", m.name);
            for f in m.fields {
                assert!(
                    f.offset + f.ty.size() * f.array.max(1) <= m.len,
                    "{}.{}",
                    m.name,
                    f.name
                );
            }
            let info = mavlink_tlog::message_info(m.id).expect("same dialect as mavlink-tlog");
            assert_eq!(info.max_len as usize, m.len, "{}", m.name);
        }
    }
}

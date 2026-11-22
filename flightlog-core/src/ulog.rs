//! PX4 ULog (`.ulg`) reader.
//!
//! Reads a ULog file into the same typed [`Table`]s as the DataFlash and
//! tlog readers: one table per logged topic, one column per field, with
//! nested types and arrays flattened the way pyulog names them
//! (`accel_device_ids[0]`, `estimator_status.pos_horiz_accuracy`). Every
//! table has a `multi_id` column ([`Table::instance_field`]) so
//! `table.instance(1)` gives the second instance of a topic.
//!
//! Besides the data it keeps what a flight review needs: the info fields
//! (`sys_name`, `ver_sw`, ...), the parameters at start and every change
//! in flight, the system and airframe defaults, the logged console
//! messages, and the dropouts the logger itself reported.
//!
//! Damaged files are read the way pyulog reads them: an implausible
//! message header (type 0, size 0 or over 10,000 bytes) makes the reader
//! search for the next ULog sync sequence, a data message with the wrong
//! size for its topic is skipped, and data appended after a crash
//! (`DATA_APPENDED` flag) is read from its offset. Unlike pyulog, every
//! byte that could not be used is counted in [`ULog::integrity`].
//!
//! ```no_run
//! use flightlog_core::ulog::ULog;
//!
//! let log = ULog::open("log_0_2026-11-22-10-01-02.ulg")?;
//! let att = log.table("vehicle_attitude").expect("attitude logged");
//! let q0 = att.f64s("q[0]").unwrap();
//! println!("{} attitude rows, MPC_XY_VEL_MAX = {:?}", q0.len(), log.param("MPC_XY_VEL_MAX"));
//! for m in &log.logged {
//!     println!("{} {} {}", m.timestamp_us, m.level_name(), m.text);
//! }
//! # Ok::<(), flightlog_core::Error>(())
//! ```

use std::collections::{BTreeMap, HashMap};

use crate::column::{Column, ColumnData, Table};
use crate::log::Integrity;

/// The first 7 bytes of every ULog file.
pub const MAGIC: [u8; 7] = [b'U', b'L', b'o', b'g', 0x01, 0x12, 0x35];

/// The ULog sync sequence, written so readers can recover from damage.
pub const SYNC: [u8; 8] = [0x2F, 0x73, 0x13, 0x20, 0x25, 0x0C, 0xBB, 0x12];

/// Message headers larger than this are treated as damage (pyulog rule).
const MAX_PLAUSIBLE_MSG: usize = 10_000;
/// Deepest nesting of message formats that is followed.
const MAX_NESTING: usize = 16;
/// Largest flattened topic, in bytes (a data message cannot be larger).
const MAX_TOPIC_BYTES: usize = 65_535;

/// True when `bytes` starts with the ULog magic.
pub fn is_ulog(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && bytes[..MAGIC.len()] == MAGIC
}

/// A value of an info message (`I` or `M`).
#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    /// `char[N]`.
    Text(String),
    /// Signed integer types.
    Int(i64),
    /// Unsigned integer types.
    UInt(u64),
    /// `float` or `double`.
    Float(f64),
    /// `bool`.
    Bool(bool),
    /// Arrays and other types, as raw bytes.
    Bytes(Vec<u8>),
}

impl InfoValue {
    /// The text, for `char[N]` values.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            InfoValue::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The value as `f64`, for numbers and bools.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            InfoValue::Int(v) => Some(v as f64),
            InfoValue::UInt(v) => Some(v as f64),
            InfoValue::Float(v) => Some(v),
            InfoValue::Bool(v) => Some(v as u8 as f64),
            _ => None,
        }
    }
}

/// A parameter change logged during the flight.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamChange {
    /// Timestamp of the last data message before the change (microseconds since boot).
    pub timestamp_us: u64,
    /// Parameter name.
    pub name: String,
    /// New value.
    pub value: f64,
}

/// Default values of one parameter (`Q` messages).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ParamDefault {
    /// The firmware's default.
    pub system: Option<f64>,
    /// The default of the current configuration (airframe).
    pub airframe: Option<f64>,
}

/// A console message the firmware logged (`L` or `C`).
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedMessage {
    /// Microseconds since boot.
    pub timestamp_us: u64,
    /// Syslog level, 0 (emergency) to 7 (debug).
    pub level: u8,
    /// Tag of a tagged message (`C`), `None` for `L`.
    pub tag: Option<u16>,
    /// The text.
    pub text: String,
}

impl LoggedMessage {
    /// `"EMERGENCY"`, `"ALERT"`, `"CRITICAL"`, `"ERROR"`, `"WARNING"`,
    /// `"NOTICE"`, `"INFO"`, `"DEBUG"` or `"UNKNOWN"`.
    pub fn level_name(&self) -> &'static str {
        match self.level {
            0 => "EMERGENCY",
            1 => "ALERT",
            2 => "CRITICAL",
            3 => "ERROR",
            4 => "WARNING",
            5 => "NOTICE",
            6 => "INFO",
            7 => "DEBUG",
            _ => "UNKNOWN",
        }
    }
}

/// A gap the logger reported itself (`O` message): it could not keep up
/// and dropped data for `duration_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dropout {
    /// Timestamp of the last data message before the gap.
    pub timestamp_us: u64,
    /// Length of the gap in milliseconds.
    pub duration_ms: u16,
}

/// A PX4 ULog file.
#[derive(Debug, Clone)]
pub struct ULog {
    /// ULog file format version from the header.
    pub version: u8,
    /// Timestamp in the file header (microseconds since boot).
    pub start_timestamp_us: u64,
    /// Compatible flag bits (`B` message), zero when absent.
    pub compat_flags: [u8; 8],
    /// Incompatible flag bits (`B` message). Bit 0 of byte 0 is
    /// `DATA_APPENDED`; any other set bit is a format this reader does not
    /// know, and the file is read anyway.
    pub incompat_flags: [u8; 8],
    /// File offsets of appended data (non-zero entries only).
    pub appended_offsets: Vec<u64>,
    /// Info messages (`I`), by key without the type: `sys_name`, `ver_hw`,
    /// `ver_sw`, `sys_uuid`, ...
    pub info: BTreeMap<String, InfoValue>,
    /// Multi-part info messages (`M`), by key: each entry is one value
    /// assembled from its continued parts.
    pub info_multiple: BTreeMap<String, Vec<Vec<InfoValue>>>,
    /// Parameters as they were when logging started.
    pub initial_params: BTreeMap<String, f64>,
    /// Parameters changed while logging, in file order.
    pub changed_params: Vec<ParamChange>,
    /// Parameters at the end of the log (initial values with every change applied).
    pub params: BTreeMap<String, f64>,
    /// System and airframe defaults (`Q` messages), by parameter name.
    pub default_params: BTreeMap<String, ParamDefault>,
    /// Console messages (`L` and `C`), in file order.
    pub logged: Vec<LoggedMessage>,
    /// Gaps the logger reported.
    pub dropouts: Vec<Dropout>,
    /// Sync messages seen.
    pub sync_count: u64,
    /// Messages per topic name (all instances), for every topic with data.
    pub counts: BTreeMap<String, u64>,
    /// What the reader had to skip. `unknown_types` counts data messages
    /// whose message id was never subscribed (`A`), by id;
    /// `invalid_formats` lists formats and subscriptions that could not be used.
    pub integrity: Integrity,
    tables: Vec<Table>,
    index: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prim {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Bool,
    Char,
}

impl Prim {
    fn parse(s: &str) -> Option<Prim> {
        Some(match s {
            "int8_t" => Prim::I8,
            "uint8_t" => Prim::U8,
            "int16_t" => Prim::I16,
            "uint16_t" => Prim::U16,
            "int32_t" => Prim::I32,
            "uint32_t" => Prim::U32,
            "int64_t" => Prim::I64,
            "uint64_t" => Prim::U64,
            "float" => Prim::F32,
            "double" => Prim::F64,
            "bool" => Prim::Bool,
            "char" => Prim::Char,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Prim::I8 | Prim::U8 | Prim::Bool | Prim::Char => 1,
            Prim::I16 | Prim::U16 => 2,
            Prim::I32 | Prim::U32 | Prim::F32 => 4,
            Prim::I64 | Prim::U64 | Prim::F64 => 8,
        }
    }

    // pyulog stores bool and char as int8
    fn empty(self) -> ColumnData {
        match self {
            Prim::I8 | Prim::Bool | Prim::Char => ColumnData::Int8(Vec::new()),
            Prim::U8 => ColumnData::UInt8(Vec::new()),
            Prim::I16 => ColumnData::Int16(Vec::new()),
            Prim::U16 => ColumnData::UInt16(Vec::new()),
            Prim::I32 => ColumnData::Int32(Vec::new()),
            Prim::U32 => ColumnData::UInt32(Vec::new()),
            Prim::I64 => ColumnData::Int64(Vec::new()),
            Prim::U64 => ColumnData::UInt64(Vec::new()),
            Prim::F32 => ColumnData::Float32(Vec::new()),
            Prim::F64 => ColumnData::Float64(Vec::new()),
        }
    }
}

#[derive(Debug, Clone)]
struct FieldSpec {
    ty: String,
    array: usize,
    name: String,
}

#[derive(Debug, Clone)]
struct Flat {
    name: String,
    prim: Prim,
    offset: usize,
}

/// A topic layout: flattened fields and accepted data sizes.
#[derive(Debug)]
struct Layout {
    fields: Vec<Flat>,
    /// Size without trailing padding (smallest data message accepted).
    min_size: usize,
    /// Size with trailing padding.
    max_size: usize,
    timestamp: Option<usize>,
}

fn parse_format(text: &str) -> Option<(String, Vec<FieldSpec>)> {
    let (name, rest) = text.split_once(':')?;
    let mut fields = Vec::new();
    for f in rest.split(';').filter(|f| !f.is_empty()) {
        let (ty, fname) = f.split_once(' ')?;
        let (ty, array) = match ty.find('[') {
            Some(a) => {
                let b = ty.find(']')?;
                let n: usize = ty.get(a + 1..b)?.parse().ok()?;
                if n > 0xFFFF {
                    return None;
                }
                (&ty[..a], n)
            }
            None => (ty, 0),
        };
        fields.push(FieldSpec {
            ty: ty.to_string(),
            array,
            name: fname.trim().to_string(),
        });
    }
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), fields))
}

fn flatten(
    formats: &HashMap<String, Vec<FieldSpec>>,
    ty: &str,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, Prim)>,
    bytes: &mut usize,
) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("{ty}: nested deeper than {MAX_NESTING}"));
    }
    let fields = formats
        .get(ty)
        .ok_or_else(|| format!("no format for type {ty}"))?;
    for f in fields {
        let elems: Vec<String> = if f.array > 0 {
            (0..f.array)
                .map(|i| format!("{prefix}{}[{i}]", f.name))
                .collect()
        } else {
            vec![format!("{prefix}{}", f.name)]
        };
        match Prim::parse(&f.ty) {
            Some(p) => {
                for e in elems {
                    *bytes += p.size();
                    if *bytes > MAX_TOPIC_BYTES {
                        return Err(format!("{ty}: larger than {MAX_TOPIC_BYTES} bytes"));
                    }
                    out.push((e, p));
                }
            }
            None => {
                for e in elems {
                    flatten(formats, &f.ty, &format!("{e}."), depth + 1, out, bytes)?;
                }
            }
        }
    }
    Ok(())
}

fn layout(formats: &HashMap<String, Vec<FieldSpec>>, name: &str) -> Result<Layout, String> {
    let mut flat = Vec::new();
    let mut bytes = 0;
    flatten(formats, name, "", 0, &mut flat, &mut bytes)?;
    let max_size = bytes;
    // trailing padding may be left out of data messages
    while flat.last().is_some_and(|(n, _)| n.starts_with("_padding")) {
        flat.pop();
    }
    let mut fields = Vec::new();
    let mut offset = 0;
    for (n, p) in flat {
        if !n.starts_with("_padding") {
            fields.push(Flat {
                name: n,
                prim: p,
                offset,
            });
        }
        offset += p.size();
    }
    let timestamp = fields
        .iter()
        .find(|f| f.name == "timestamp" && f.prim == Prim::U64)
        .map(|f| f.offset);
    Ok(Layout {
        fields,
        min_size: offset,
        max_size,
        timestamp,
    })
}

struct Builder {
    name: String,
    layout: std::rc::Rc<Layout>,
    cols: Vec<ColumnData>,
    multi: Vec<u8>,
}

impl Builder {
    fn push(&mut self, d: &[u8], multi_id: u8) {
        for (f, c) in self.layout.fields.iter().zip(self.cols.iter_mut()) {
            let b = &d[f.offset..f.offset + f.prim.size()];
            match c {
                ColumnData::Int8(v) => v.push(b[0] as i8),
                ColumnData::UInt8(v) => v.push(b[0]),
                ColumnData::Int16(v) => v.push(i16::from_le_bytes([b[0], b[1]])),
                ColumnData::UInt16(v) => v.push(u16::from_le_bytes([b[0], b[1]])),
                ColumnData::Int32(v) => v.push(i32::from_le_bytes(b.try_into().expect("4"))),
                ColumnData::UInt32(v) => v.push(u32::from_le_bytes(b.try_into().expect("4"))),
                ColumnData::Int64(v) => v.push(i64::from_le_bytes(b.try_into().expect("8"))),
                ColumnData::UInt64(v) => v.push(u64::from_le_bytes(b.try_into().expect("8"))),
                ColumnData::Float32(v) => v.push(f32::from_le_bytes(b.try_into().expect("4"))),
                ColumnData::Float64(v) => v.push(f64::from_le_bytes(b.try_into().expect("8"))),
                _ => unreachable!("built from Prim::empty"),
            }
        }
        self.multi.push(multi_id);
    }

    fn finish(self) -> Table {
        let rows = self.multi.len();
        let mut columns: Vec<Column> = self
            .layout
            .fields
            .iter()
            .zip(self.cols)
            .map(|(f, data)| Column {
                name: f.name.clone(),
                unit: None,
                multiplier: None,
                data,
            })
            .collect();
        columns.push(Column {
            name: "multi_id".into(),
            unit: None,
            multiplier: None,
            data: ColumnData::UInt8(self.multi),
        });
        Table {
            name: self.name,
            columns,
            instance_field: Some("multi_id".into()),
            format: None,
            rows,
        }
    }
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// Parses `key_len, "type name", value` into (type, name, value).
fn typed_key(b: &[u8]) -> Option<(String, String, &[u8])> {
    let kl = *b.first()? as usize;
    let key = std::str::from_utf8(b.get(1..1 + kl)?).ok()?;
    let (ty, name) = key.split_once(' ')?;
    Some((ty.to_string(), name.to_string(), &b[1 + kl..]))
}

fn info_value(ty: &str, v: &[u8]) -> InfoValue {
    if ty.starts_with("char[") {
        return InfoValue::Text(text(v));
    }
    let Some(p) = Prim::parse(ty) else {
        return InfoValue::Bytes(v.to_vec());
    };
    if v.len() != p.size() {
        return InfoValue::Bytes(v.to_vec());
    }
    match p {
        Prim::I8 => InfoValue::Int(v[0] as i8 as i64),
        Prim::Char => InfoValue::Int(v[0] as i8 as i64),
        Prim::U8 => InfoValue::UInt(v[0] as u64),
        Prim::Bool => InfoValue::Bool(v[0] != 0),
        Prim::I16 => InfoValue::Int(i16::from_le_bytes([v[0], v[1]]) as i64),
        Prim::U16 => InfoValue::UInt(u16::from_le_bytes([v[0], v[1]]) as u64),
        Prim::I32 => InfoValue::Int(i32::from_le_bytes(v.try_into().expect("4")) as i64),
        Prim::U32 => InfoValue::UInt(u32::from_le_bytes(v.try_into().expect("4")) as u64),
        Prim::I64 => InfoValue::Int(i64::from_le_bytes(v.try_into().expect("8"))),
        Prim::U64 => InfoValue::UInt(u64::from_le_bytes(v.try_into().expect("8"))),
        Prim::F32 => InfoValue::Float(crate::short_f32(f32::from_le_bytes(
            v.try_into().expect("4"),
        ))),
        Prim::F64 => InfoValue::Float(f64::from_le_bytes(v.try_into().expect("8"))),
    }
}

/// A parameter message (`P`, or `Q` without its first byte) as (name, value).
fn param_value(b: &[u8]) -> Option<(String, f64)> {
    let (ty, name, v) = typed_key(b)?;
    let v: [u8; 4] = v.get(..4)?.try_into().ok()?;
    let value = match ty.as_str() {
        "int32_t" => i32::from_le_bytes(v) as f64,
        "float" => crate::short_f32(f32::from_le_bytes(v)),
        _ => return None,
    };
    Some((name, value))
}

fn find_sync(bytes: &[u8], from: usize, to: usize) -> Option<usize> {
    let to = to.min(bytes.len());
    if from >= to {
        return None;
    }
    bytes[from..to]
        .windows(SYNC.len())
        .position(|w| w == SYNC)
        .map(|i| from + i)
}

impl ULog {
    /// Reads a ULog file from memory.
    ///
    /// Returns [`Error::UnknownFormat`](crate::Error::UnknownFormat) when the
    /// bytes do not start with the ULog magic.
    pub fn read(bytes: &[u8]) -> Result<ULog, crate::Error> {
        if !is_ulog(bytes) || bytes.len() < 16 {
            return Err(crate::Error::UnknownFormat);
        }
        let mut log = ULog {
            version: bytes[7],
            start_timestamp_us: u64_at(bytes, 8).unwrap_or(0),
            compat_flags: [0; 8],
            incompat_flags: [0; 8],
            appended_offsets: Vec::new(),
            info: BTreeMap::new(),
            info_multiple: BTreeMap::new(),
            initial_params: BTreeMap::new(),
            changed_params: Vec::new(),
            params: BTreeMap::new(),
            default_params: BTreeMap::new(),
            logged: Vec::new(),
            dropouts: Vec::new(),
            sync_count: 0,
            counts: BTreeMap::new(),
            integrity: Integrity {
                bytes: bytes.len() as u64,
                ..Integrity::default()
            },
            tables: Vec::new(),
            index: HashMap::new(),
        };

        let mut formats: HashMap<String, Vec<FieldSpec>> = HashMap::new();
        let mut layouts: HashMap<String, std::rc::Rc<Layout>> = HashMap::new();
        let mut builders: Vec<Builder> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        // msg_id -> (builder, multi_id)
        let mut subs: HashMap<u16, (usize, u8)> = HashMap::new();
        let mut in_definitions = true;
        let mut has_sync = true;
        let mut last_ts = log.start_timestamp_us;
        let mut appended: Vec<usize> = Vec::new();
        let mut next_appended = 0;

        let mut pos = 16usize;
        // end of the last message that was used; bytes between it and the
        // next used message were skipped
        let mut used_end = 16usize;
        let n = bytes.len();

        loop {
            // data appended after a crash continues at its offset
            if let Some(&off) = appended.get(next_appended) {
                if pos >= off {
                    next_appended += 1;
                    continue;
                }
            }
            if pos + 3 > n {
                break;
            }
            let size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            let ty = bytes[pos + 2];
            let end = pos + 3 + size;

            if ty == 0 || size == 0 || size > MAX_PLAUSIBLE_MSG {
                // damage: resynchronize on the next sync sequence
                let from = pos + 1;
                match has_sync.then(|| find_sync(bytes, from, n)).flatten() {
                    Some(s) => pos = s + SYNC.len(),
                    None => {
                        has_sync = false;
                        pos = from;
                    }
                }
                continue;
            }
            if end > n {
                break;
            }
            if let Some(&off) = appended.get(next_appended) {
                if end > off {
                    // the message runs into the appended data: it was cut
                    // off by the crash, and reading continues at the offset
                    pos = off;
                    next_appended += 1;
                    continue;
                }
            }
            let p = &bytes[pos + 3..end];

            let ok = match ty {
                b'B' => {
                    if pos == 16 && p.len() >= 40 {
                        log.compat_flags.copy_from_slice(&p[..8]);
                        log.incompat_flags.copy_from_slice(&p[8..16]);
                        for i in 0..3 {
                            let o = u64_at(p, 16 + i * 8).unwrap_or(0);
                            if o != 0 {
                                log.appended_offsets.push(o);
                            }
                        }
                        if log.incompat_flags[0] & 1 != 0 {
                            let mut prev = end;
                            for &o in &log.appended_offsets {
                                // offsets that point backwards or past the end are damage
                                if o as usize >= prev && (o as usize) <= n {
                                    appended.push(o as usize);
                                    prev = o as usize;
                                }
                            }
                        }
                        true
                    } else {
                        false
                    }
                }
                b'F' => match parse_format(&text(p)) {
                    Some((name, fields)) => {
                        formats.insert(name, fields);
                        true
                    }
                    None => {
                        log.integrity
                            .invalid_formats
                            .push(format!("unreadable format: {}", text(p)));
                        false
                    }
                },
                b'I' => match typed_key(p) {
                    Some((ty, key, v)) => {
                        log.info.insert(key, info_value(&ty, v));
                        true
                    }
                    None => false,
                },
                b'M' => match p.split_first().and_then(|(&c, r)| Some((c, typed_key(r)?))) {
                    Some((cont, (ty, key, v))) => {
                        let v = info_value(&ty, v);
                        let e = log.info_multiple.entry(key).or_default();
                        match e.last_mut() {
                            Some(last) if cont != 0 => last.push(v),
                            _ => e.push(vec![v]),
                        }
                        true
                    }
                    None => false,
                },
                b'P' => match param_value(p) {
                    Some((name, value)) => {
                        if in_definitions {
                            log.initial_params.insert(name, value);
                        } else {
                            log.changed_params.push(ParamChange {
                                timestamp_us: last_ts,
                                name,
                                value,
                            });
                        }
                        true
                    }
                    None => false,
                },
                b'Q' => match p
                    .split_first()
                    .and_then(|(&t, r)| Some((t, param_value(r)?)))
                {
                    Some((types, (name, value))) => {
                        let d = log.default_params.entry(name).or_default();
                        if types & 1 != 0 {
                            d.system = Some(value);
                        }
                        if types & 2 != 0 {
                            d.airframe = Some(value);
                        }
                        true
                    }
                    None => false,
                },
                b'A' => {
                    in_definitions = false;
                    match (p.first(), u16_at(p, 1)) {
                        (Some(&multi), Some(id)) => {
                            let name = text(&p[3..]);
                            let lay = match layouts.get(&name) {
                                Some(l) => Ok(l.clone()),
                                None => layout(&formats, &name).map(|l| {
                                    let l = std::rc::Rc::new(l);
                                    layouts.insert(name.clone(), l.clone());
                                    l
                                }),
                            };
                            match lay {
                                Ok(lay) => {
                                    let b = *by_name.entry(name.clone()).or_insert_with(|| {
                                        builders.push(Builder {
                                            name: name.clone(),
                                            cols: lay
                                                .fields
                                                .iter()
                                                .map(|f| f.prim.empty())
                                                .collect(),
                                            layout: lay.clone(),
                                            multi: Vec::new(),
                                        });
                                        builders.len() - 1
                                    });
                                    subs.insert(id, (b, multi));
                                    true
                                }
                                Err(e) => {
                                    log.integrity.invalid_formats.push(e);
                                    false
                                }
                            }
                        }
                        _ => false,
                    }
                }
                b'R' => match u16_at(p, 0) {
                    Some(id) => {
                        subs.remove(&id);
                        true
                    }
                    None => false,
                },
                b'D' => match u16_at(p, 0) {
                    Some(id) => match subs.get(&id) {
                        Some(&(b, multi)) => {
                            let d = &p[2..];
                            let bl = &mut builders[b];
                            if d.len() >= bl.layout.min_size && d.len() <= bl.layout.max_size {
                                if let Some(t) = bl.layout.timestamp.and_then(|o| u64_at(d, o)) {
                                    if t > last_ts {
                                        last_ts = t;
                                    }
                                }
                                bl.push(d, multi);
                                true
                            } else {
                                false
                            }
                        }
                        None => {
                            *log.integrity.unknown_types.entry(id as u32).or_insert(0) += 1;
                            false
                        }
                    },
                    None => false,
                },
                b'L' | b'C' => {
                    in_definitions = false;
                    let tagged = ty == b'C';
                    let ts_at = if tagged { 3 } else { 1 };
                    match (p.first(), u64_at(p, ts_at)) {
                        (Some(&lvl), Some(t)) => {
                            log.logged.push(LoggedMessage {
                                timestamp_us: t,
                                // PX4 writes the level as an ASCII digit
                                level: lvl.wrapping_sub(b'0'),
                                tag: if tagged { u16_at(p, 1) } else { None },
                                text: text(&p[ts_at + 8..]),
                            });
                            true
                        }
                        _ => false,
                    }
                }
                b'S' => {
                    log.sync_count += 1;
                    true
                }
                b'O' => match u16_at(p, 0) {
                    Some(d) => {
                        log.dropouts.push(Dropout {
                            timestamp_us: last_ts,
                            duration_ms: d,
                        });
                        true
                    }
                    None => false,
                },
                _ => {
                    // A message type this reader does not know: skip it, but
                    // a sync sequence inside it means the header was damage.
                    match has_sync.then(|| find_sync(bytes, pos + 3, end)).flatten() {
                        Some(s) => {
                            pos = s + SYNC.len();
                            continue;
                        }
                        None => true,
                    }
                }
            };
            if ok {
                if pos > used_end {
                    log.integrity.skip(used_end as u64, (pos - used_end) as u64);
                }
                if ty != b'S' {
                    log.integrity.messages += 1;
                }
                used_end = end;
            }
            pos = end;
        }

        // bytes after the last used message
        let tail_start = pos.min(n);
        if tail_start > used_end {
            log.integrity
                .skip(used_end as u64, (tail_start - used_end) as u64);
        }
        log.integrity.truncated_tail_bytes = (n - tail_start) as u64;

        log.params = log.initial_params.clone();
        for c in &log.changed_params {
            log.params.insert(c.name.clone(), c.value);
        }
        for b in builders {
            if b.multi.is_empty() {
                continue;
            }
            log.counts.insert(b.name.clone(), b.multi.len() as u64);
            log.index.insert(b.name.clone(), log.tables.len());
            log.tables.push(b.finish());
        }
        Ok(log)
    }

    /// Reads a ULog file from disk.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<ULog, crate::Error> {
        ULog::read(&std::fs::read(path)?)
    }

    /// The table of one topic (all instances; see [`Table::instance`]).
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.index.get(name).map(|&i| &self.tables[i])
    }

    /// All tables, in the order their topics were first subscribed.
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// Topic names that have data.
    pub fn names(&self) -> Vec<&str> {
        self.tables.iter().map(|t| t.name.as_str()).collect()
    }

    /// Parameter value at the end of the log.
    pub fn param(&self, name: &str) -> Option<f64> {
        self.params.get(name).copied()
    }

    /// Parameters whose value at the end of the log differs from the
    /// firmware default (`Q` messages), as (name, value, default). Empty
    /// when the log has no defaults (PX4 before v1.12).
    pub fn non_default_params(&self) -> Vec<(&str, f64, f64)> {
        self.params
            .iter()
            .filter_map(|(k, &v)| {
                let d = self.default_params.get(k)?.system?;
                (d != v).then_some((k.as_str(), v, d))
            })
            .collect()
    }

    /// An info text value (`sys_name`, `ver_hw`, `ver_sw`, ...).
    pub fn info_str(&self, key: &str) -> Option<&str> {
        self.info.get(key)?.as_str()
    }

    /// First and last timestamp of the data, microseconds since boot, by
    /// the same rule as [`Log::time_range_us`](crate::Log::time_range_us):
    /// isolated (damaged) timestamps do not stretch the range.
    pub fn time_range_us(&self) -> Option<(u64, u64)> {
        crate::log::table_time_range(&self.tables, "timestamp")
    }

    /// Unix microseconds at timestamp 0, from the GPS topic's
    /// `time_utc_usec` (`None` without a GPS time).
    pub fn utc_offset_us(&self) -> Option<i64> {
        for name in ["vehicle_gps_position", "sensor_gps"] {
            let Some(t) = self.table(name) else { continue };
            let (Some(ts), Some(utc)) = (t.column("timestamp"), t.column("time_utc_usec")) else {
                continue;
            };
            for i in 0..t.len() {
                let (Some(a), Some(u)) = (ts.data.get_i64(i), utc.data.get_i64(i)) else {
                    continue;
                };
                if u > 0 && a > 0 {
                    return Some(u - a);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parsing() {
        let (n, f) =
            parse_format("vehicle_status:uint64_t timestamp;float[4] q;uint8_t[2] _padding0;")
                .unwrap();
        assert_eq!(n, "vehicle_status");
        assert_eq!(f.len(), 3);
        assert_eq!(f[1].array, 4);
        assert!(parse_format("x:float[99999999] a;").is_none());
        assert!(parse_format("no colon").is_none());
    }

    #[test]
    fn recursive_formats_are_rejected() {
        let mut formats = HashMap::new();
        formats.insert(
            "a".to_string(),
            vec![FieldSpec {
                ty: "a".into(),
                array: 0,
                name: "self".into(),
            }],
        );
        assert!(layout(&formats, "a").is_err());
    }

    #[test]
    fn huge_topics_are_rejected() {
        let mut formats = HashMap::new();
        formats.insert(
            "inner".to_string(),
            vec![FieldSpec {
                ty: "double".into(),
                array: 0xFFFF,
                name: "v".into(),
            }],
        );
        formats.insert(
            "outer".to_string(),
            vec![FieldSpec {
                ty: "inner".into(),
                array: 0xFFFF,
                name: "x".into(),
            }],
        );
        assert!(layout(&formats, "outer").is_err());
    }
}

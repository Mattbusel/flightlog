//! DataFlash message formats (FMT) and field decoding.

use std::collections::HashMap;

use crate::column::{ColumnData, DataType, VarData};

/// Bytes taken by one format character, `None` for an unknown character.
///
/// The characters are those of ArduPilot's `AP_Logger/LogStructure.h` and
/// pymavlink's `DFReader.FORMAT_TO_STRUCT`.
pub fn field_size(c: u8) -> Option<usize> {
    Some(match c {
        b'b' | b'B' | b'M' => 1,
        b'h' | b'H' | b'c' | b'C' | b'g' => 2,
        b'i' | b'I' | b'e' | b'E' | b'f' | b'L' | b'n' => 4,
        b'q' | b'Q' | b'd' => 8,
        b'N' => 16,
        b'a' | b'Z' => 64,
        _ => return None,
    })
}

fn scaled(c: u8) -> bool {
    matches!(c, b'c' | b'C' | b'e' | b'E' | b'L')
}

/// One message type definition.
#[derive(Debug, Clone, PartialEq)]
pub struct Format {
    /// Type id (the third header byte).
    pub type_id: u8,
    /// Total message length including the 3 byte header.
    pub len: usize,
    /// Message name.
    pub name: String,
    /// Format string, one character per field.
    pub format: String,
    /// Field labels (duplicates get a `_2`, `_3` suffix). Only fields that
    /// have both a format character and a label are kept.
    pub labels: Vec<String>,
    offsets: Vec<usize>,
    chars: Vec<u8>,
    usable: bool,
}

impl Format {
    /// The built-in definition of FMT itself.
    pub fn bootstrap_fmt() -> Format {
        Format::new(
            0x80,
            89,
            "FMT".into(),
            "BBnNZ".into(),
            "Type,Length,Name,Format,Columns",
        )
        .expect("valid")
    }

    fn new(
        type_id: u8,
        len: usize,
        name: String,
        format: String,
        columns: &str,
    ) -> Result<Format, String> {
        let chars: Vec<u8> = format.bytes().collect();
        let mut offsets = Vec::with_capacity(chars.len());
        let mut ofs = 0usize;
        for &c in &chars {
            let Some(sz) = field_size(c) else {
                return Err(format!(
                    "{name}: unsupported format character {:?}",
                    c as char
                ));
            };
            offsets.push(ofs);
            ofs += sz;
        }
        let usable = len >= 3 && ofs == len - 3;
        let raw: Vec<&str> = if columns.is_empty() {
            Vec::new()
        } else {
            columns.split(',').collect()
        };
        let n = raw.len().min(chars.len());
        let mut labels: Vec<String> = Vec::with_capacity(n);
        for l in raw.iter().take(n) {
            let mut l = (*l).to_string();
            if labels.contains(&l) {
                let mut k = 2;
                while labels.contains(&format!("{l}_{k}")) {
                    k += 1;
                }
                l = format!("{l}_{k}");
            }
            labels.push(l);
        }
        Ok(Format {
            type_id,
            len,
            name,
            format,
            labels,
            offsets,
            chars,
            usable,
        })
    }

    /// Parses the body of a FMT message.
    pub fn from_fmt_body(body: &[u8]) -> Result<Format, String> {
        if body.len() < 86 {
            return Err("FMT body too short".into());
        }
        let type_id = body[0];
        let len = body[1] as usize;
        let name = decode_text(&body[2..6]);
        let format = decode_text(&body[6..22]);
        let columns = decode_text(&body[22..86]);
        Format::new(type_id, len, name, format, &columns)
    }

    /// False when the format string's size does not match the length, so
    /// messages of this type cannot be decoded (pymavlink fails on them too).
    pub fn is_usable(&self) -> bool {
        self.usable
    }

    /// Same name, length, format and labels.
    pub fn same_layout(&self, other: &Format) -> bool {
        self.len == other.len
            && self.name == other.name
            && self.format == other.format
            && self.labels == other.labels
    }

    fn index(&self, label: &str) -> Option<usize> {
        self.labels.iter().position(|l| l == label)
    }

    /// A numeric field as `f64` without format multipliers.
    pub fn num(&self, body: &[u8], label: &str) -> Option<f64> {
        let i = self.index(label)?;
        let (c, o) = (self.chars[i], self.offsets[i]);
        let b = body.get(o..o + field_size(c)?)?;
        Some(match c {
            b'b' => b[0] as i8 as f64,
            b'B' | b'M' => b[0] as f64,
            b'h' | b'c' => i16::from_le_bytes([b[0], b[1]]) as f64,
            b'H' | b'C' => u16::from_le_bytes([b[0], b[1]]) as f64,
            b'i' | b'e' | b'L' => i32::from_le_bytes(b.try_into().ok()?) as f64,
            b'I' | b'E' => u32::from_le_bytes(b.try_into().ok()?) as f64,
            b'q' => i64::from_le_bytes(b.try_into().ok()?) as f64,
            b'Q' => u64::from_le_bytes(b.try_into().ok()?) as f64,
            b'f' => f32::from_le_bytes(b.try_into().ok()?) as f64,
            b'd' => f64::from_le_bytes(b.try_into().ok()?),
            b'g' => f16_to_f32(u16::from_le_bytes([b[0], b[1]])) as f64,
            _ => return None,
        })
    }

    /// True when the field is a 32-bit float (`f`).
    pub fn is_f32(&self, label: &str) -> bool {
        self.index(label).is_some_and(|i| self.chars[i] == b'f')
    }

    /// A text field (`n`, `N`, `Z`), decoded as pymavlink does.
    pub fn text(&self, body: &[u8], label: &str) -> Option<String> {
        let i = self.index(label)?;
        let (c, o) = (self.chars[i], self.offsets[i]);
        if !matches!(c, b'n' | b'N' | b'Z') {
            return None;
        }
        Some(decode_text(body.get(o..o + field_size(c)?)?))
    }

    fn column_type(&self, i: usize, scale: bool) -> (DataType, usize) {
        let c = self.chars[i];
        if scale && scaled(c) {
            return (DataType::Float64, 0);
        }
        let ty = match c {
            b'b' => DataType::Int8,
            b'B' | b'M' => DataType::UInt8,
            b'h' | b'c' => DataType::Int16,
            b'H' | b'C' => DataType::UInt16,
            b'i' | b'e' | b'L' => DataType::Int32,
            b'I' | b'E' => DataType::UInt32,
            b'q' => DataType::Int64,
            b'Q' => DataType::UInt64,
            b'f' | b'g' => DataType::Float32,
            b'd' => DataType::Float64,
            b'Z' if self.name == "FILE" => DataType::Binary,
            b'n' | b'N' | b'Z' => DataType::Utf8,
            b'a' => return (DataType::Int16, 32),
            _ => unreachable!("validated"),
        };
        (ty, 0)
    }

    /// Element type and list size of each labeled column.
    pub fn column_types(&self, scale: bool) -> Vec<(DataType, usize)> {
        (0..self.labels.len())
            .map(|i| self.column_type(i, scale))
            .collect()
    }

    pub(crate) fn empty_columns(&self, scale: bool) -> Vec<ColumnData> {
        self.column_types(scale)
            .into_iter()
            .map(|(t, l)| ColumnData::empty(t, l))
            .collect()
    }

    /// Appends one message body to the columns made by `empty_columns`.
    pub(crate) fn decode_into(&self, body: &[u8], cols: &mut [ColumnData], scale: bool) {
        for (i, col) in cols.iter_mut().enumerate() {
            let c = self.chars[i];
            let o = self.offsets[i];
            let b = &body[o..o + field_size(c).expect("validated")];
            push(col, c, b, scale);
        }
    }

    /// Units and multipliers per labeled column from FMTU ids.
    pub(crate) fn units(
        &self,
        unit_ids: &str,
        mult_ids: &str,
        unit_lookup: &HashMap<u8, String>,
        mult_lookup: &HashMap<u8, f64>,
    ) -> Vec<(Option<String>, Option<f64>)> {
        let uids = unit_ids.as_bytes();
        let mids = mult_ids.as_bytes();
        (0..self.labels.len())
            .map(|i| {
                let mut unit = uids
                    .get(i)
                    .and_then(|u| unit_lookup.get(u))
                    .cloned()
                    .unwrap_or_default();
                let mult = mids.get(i).and_then(|m| mult_lookup.get(m)).copied();
                if !scaled(self.chars[i]) && !unit.is_empty() {
                    if let Some(m) = mult {
                        unit = match mult_prefix(m) {
                            Some(p) => format!("{p}{unit}"),
                            None => format!("{} {unit}", py_g4(m)),
                        };
                    }
                }
                ((!unit.is_empty()).then_some(unit), mult)
            })
            .collect()
    }
}

fn mult_prefix(m: f64) -> Option<&'static str> {
    const PREFIXES: [(f64, &str); 7] = [
        (0.0, ""),
        (1.0, ""),
        (1e-1, "d"),
        (1e-2, "c"),
        (1e-3, "m"),
        (1e-6, "\u{b5}"),
        (1e-9, "n"),
    ];
    PREFIXES.iter().find(|(k, _)| *k == m).map(|(_, p)| *p)
}

/// Python's `"%.4g" % x`.
pub(crate) fn py_g4(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if !x.is_finite() {
        return if x.is_nan() {
            "nan".into()
        } else if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let sci = format!("{:.3e}", x);
    let (mant, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("int");
    if (-4..4).contains(&exp) {
        let decimals = (3 - exp).max(0) as usize;
        let s = format!("{:.*}", decimals, x);
        strip_zeros(&s)
    } else {
        let m = strip_zeros(mant);
        format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// pymavlink text rule: UTF-8 if the whole field decodes, else Latin-1;
/// then cut at the first NUL.
pub(crate) fn decode_text(b: &[u8]) -> String {
    let s = match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    };
    match s.find('\0') {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}

/// IEEE 754 half precision to f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let frac = (h & 0x3ff) as u32;
    let bits = match (exp, frac) {
        (0, 0) => sign,
        (0, f) => {
            // subnormal: normalize
            let mut e = 127 - 15 + 1;
            let mut f = f;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((f & 0x3ff) << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, f) => sign | 0x7f80_0000 | (f << 13),
        (e, f) => sign | ((e + 127 - 15) << 23) | (f << 13),
    };
    f32::from_bits(bits)
}

fn push(col: &mut ColumnData, c: u8, b: &[u8], scale: bool) {
    match col {
        ColumnData::Int8(v) => v.push(b[0] as i8),
        ColumnData::UInt8(v) => v.push(b[0]),
        ColumnData::Int16(v) => v.push(i16::from_le_bytes([b[0], b[1]])),
        ColumnData::UInt16(v) => v.push(u16::from_le_bytes([b[0], b[1]])),
        ColumnData::Int32(v) => v.push(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        ColumnData::UInt32(v) => v.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        ColumnData::Int64(v) => v.push(i64::from_le_bytes(b[..8].try_into().expect("8"))),
        ColumnData::UInt64(v) => v.push(u64::from_le_bytes(b[..8].try_into().expect("8"))),
        ColumnData::Float32(v) => v.push(if c == b'g' {
            f16_to_f32(u16::from_le_bytes([b[0], b[1]]))
        } else {
            f32::from_le_bytes([b[0], b[1], b[2], b[3]])
        }),
        ColumnData::Float64(v) => {
            debug_assert!(scale || c == b'd');
            v.push(match c {
                b'd' => f64::from_le_bytes(b[..8].try_into().expect("8")),
                b'c' => i16::from_le_bytes([b[0], b[1]]) as f64 / 100.0,
                b'C' => u16::from_le_bytes([b[0], b[1]]) as f64 / 100.0,
                b'e' => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 100.0,
                b'E' => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 100.0,
                b'L' => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 1e7,
                _ => unreachable!(),
            })
        }
        ColumnData::Utf8(v) => v.push(decode_text(b).as_bytes()),
        ColumnData::Binary(v) => push_bytes(v, b),
        ColumnData::FixedList { values, .. } => {
            if let ColumnData::Int16(v) = values.as_mut() {
                v.extend(b.as_chunks::<2>().0.iter().map(|p| i16::from_le_bytes(*p)));
            }
        }
    }
}

fn push_bytes(v: &mut VarData, b: &[u8]) {
    v.push(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
        assert_eq!(f16_to_f32(0x0001), 5.960_464_5e-8);
        assert!(f16_to_f32(0x7e00).is_nan());
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
    }

    #[test]
    fn g4() {
        assert_eq!(py_g4(1e-7), "1e-07");
        assert_eq!(py_g4(100.0), "100");
        assert_eq!(py_g4(1e7), "1e+07");
        assert_eq!(py_g4(0.0001), "0.0001");
        assert_eq!(py_g4(2.5), "2.5");
        assert_eq!(py_g4(12345.0), "1.234e+04");
    }

    #[test]
    fn text() {
        assert_eq!(decode_text(b"ATT\0"), "ATT");
        assert_eq!(decode_text(b"ab\0\xff"), "ab");
        assert_eq!(decode_text(b"\xe9t\xe9"), "\u{e9}t\u{e9}");
    }

    #[test]
    fn bad_char() {
        assert!(Format::new(1, 5, "X".into(), "Bx".into(), "A,B").is_err());
    }

    #[test]
    fn length_mismatch_is_unusable() {
        let f = Format::new(1, 10, "X".into(), "BB".into(), "A,B").unwrap();
        assert!(!f.is_usable());
    }
}

//! Columnar tables: one [`Table`] per message type, one typed [`Column`]
//! per field. No nulls: every message has every field.

/// Variable-length values (strings or bytes) stored back to back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarData {
    /// `offsets[i]..offsets[i + 1]` is value `i`; starts with 0.
    pub offsets: Vec<usize>,
    /// The concatenated values.
    pub data: Vec<u8>,
}

impl VarData {
    fn new() -> Self {
        VarData {
            offsets: vec![0],
            data: Vec::new(),
        }
    }

    /// Appends one value.
    pub fn push(&mut self, v: &[u8]) {
        self.data.extend_from_slice(v);
        self.offsets.push(self.data.len());
    }

    /// Number of values.
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// True when there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Value `i` as bytes.
    pub fn get(&self, i: usize) -> &[u8] {
        &self.data[self.offsets[i]..self.offsets[i + 1]]
    }
}

/// The values of one column.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `int8`
    Int8(Vec<i8>),
    /// `uint8`
    UInt8(Vec<u8>),
    /// `int16`
    Int16(Vec<i16>),
    /// `uint16`
    UInt16(Vec<u16>),
    /// `int32`
    Int32(Vec<i32>),
    /// `uint32`
    UInt32(Vec<u32>),
    /// `int64`
    Int64(Vec<i64>),
    /// `uint64`
    UInt64(Vec<u64>),
    /// `float32`
    Float32(Vec<f32>),
    /// `float64`
    Float64(Vec<f64>),
    /// UTF-8 text (always valid UTF-8).
    Utf8(VarData),
    /// Raw bytes.
    Binary(VarData),
    /// A fixed number of values per row, stored flat (`values.len() == rows * size`).
    FixedList {
        /// Values per row.
        size: usize,
        /// The flattened values.
        values: Box<ColumnData>,
    },
}

/// Logical type of a column, without data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// `int8`
    Int8,
    /// `uint8`
    UInt8,
    /// `int16`
    Int16,
    /// `uint16`
    UInt16,
    /// `int32`
    Int32,
    /// `uint32`
    UInt32,
    /// `int64`
    Int64,
    /// `uint64`
    UInt64,
    /// `float32`
    Float32,
    /// `float64`
    Float64,
    /// UTF-8 text
    Utf8,
    /// bytes
    Binary,
}

impl DataType {
    /// Lower-case name (`"float64"`, `"utf8"`, ...).
    pub fn name(self) -> &'static str {
        match self {
            DataType::Int8 => "int8",
            DataType::UInt8 => "uint8",
            DataType::Int16 => "int16",
            DataType::UInt16 => "uint16",
            DataType::Int32 => "int32",
            DataType::UInt32 => "uint32",
            DataType::Int64 => "int64",
            DataType::UInt64 => "uint64",
            DataType::Float32 => "float32",
            DataType::Float64 => "float64",
            DataType::Utf8 => "utf8",
            DataType::Binary => "binary",
        }
    }
}

impl ColumnData {
    /// An empty column of the given element type; `list` > 0 makes a fixed-size list.
    pub fn empty(ty: DataType, list: usize) -> Self {
        let inner = match ty {
            DataType::Int8 => ColumnData::Int8(Vec::new()),
            DataType::UInt8 => ColumnData::UInt8(Vec::new()),
            DataType::Int16 => ColumnData::Int16(Vec::new()),
            DataType::UInt16 => ColumnData::UInt16(Vec::new()),
            DataType::Int32 => ColumnData::Int32(Vec::new()),
            DataType::UInt32 => ColumnData::UInt32(Vec::new()),
            DataType::Int64 => ColumnData::Int64(Vec::new()),
            DataType::UInt64 => ColumnData::UInt64(Vec::new()),
            DataType::Float32 => ColumnData::Float32(Vec::new()),
            DataType::Float64 => ColumnData::Float64(Vec::new()),
            DataType::Utf8 => ColumnData::Utf8(VarData::new()),
            DataType::Binary => ColumnData::Binary(VarData::new()),
        };
        if list > 0 {
            ColumnData::FixedList {
                size: list,
                values: Box::new(inner),
            }
        } else {
            inner
        }
    }

    /// Element type (for a list, the type of its values).
    pub fn data_type(&self) -> DataType {
        match self {
            ColumnData::Int8(_) => DataType::Int8,
            ColumnData::UInt8(_) => DataType::UInt8,
            ColumnData::Int16(_) => DataType::Int16,
            ColumnData::UInt16(_) => DataType::UInt16,
            ColumnData::Int32(_) => DataType::Int32,
            ColumnData::UInt32(_) => DataType::UInt32,
            ColumnData::Int64(_) => DataType::Int64,
            ColumnData::UInt64(_) => DataType::UInt64,
            ColumnData::Float32(_) => DataType::Float32,
            ColumnData::Float64(_) => DataType::Float64,
            ColumnData::Utf8(_) => DataType::Utf8,
            ColumnData::Binary(_) => DataType::Binary,
            ColumnData::FixedList { values, .. } => values.data_type(),
        }
    }

    /// Values per row for a fixed-size list, else `None`.
    pub fn list_size(&self) -> Option<usize> {
        match self {
            ColumnData::FixedList { size, .. } => Some(*size),
            _ => None,
        }
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        match self {
            ColumnData::Int8(v) => v.len(),
            ColumnData::UInt8(v) => v.len(),
            ColumnData::Int16(v) => v.len(),
            ColumnData::UInt16(v) => v.len(),
            ColumnData::Int32(v) => v.len(),
            ColumnData::UInt32(v) => v.len(),
            ColumnData::Int64(v) => v.len(),
            ColumnData::UInt64(v) => v.len(),
            ColumnData::Float32(v) => v.len(),
            ColumnData::Float64(v) => v.len(),
            ColumnData::Utf8(v) | ColumnData::Binary(v) => v.len(),
            ColumnData::FixedList { size, values } => {
                if *size == 0 {
                    0
                } else {
                    values.len() / size
                }
            }
        }
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Row `i` as `f64`, for numeric scalar columns.
    pub fn get_f64(&self, i: usize) -> Option<f64> {
        Some(match self {
            ColumnData::Int8(v) => *v.get(i)? as f64,
            ColumnData::UInt8(v) => *v.get(i)? as f64,
            ColumnData::Int16(v) => *v.get(i)? as f64,
            ColumnData::UInt16(v) => *v.get(i)? as f64,
            ColumnData::Int32(v) => *v.get(i)? as f64,
            ColumnData::UInt32(v) => *v.get(i)? as f64,
            ColumnData::Int64(v) => *v.get(i)? as f64,
            ColumnData::UInt64(v) => *v.get(i)? as f64,
            ColumnData::Float32(v) => *v.get(i)? as f64,
            ColumnData::Float64(v) => *v.get(i)?,
            _ => return None,
        })
    }

    /// Row `i` as `i64`, for integer scalar columns (`u64` above `i64::MAX` gives `None`).
    pub fn get_i64(&self, i: usize) -> Option<i64> {
        Some(match self {
            ColumnData::Int8(v) => *v.get(i)? as i64,
            ColumnData::UInt8(v) => *v.get(i)? as i64,
            ColumnData::Int16(v) => *v.get(i)? as i64,
            ColumnData::UInt16(v) => *v.get(i)? as i64,
            ColumnData::Int32(v) => *v.get(i)? as i64,
            ColumnData::UInt32(v) => *v.get(i)? as i64,
            ColumnData::Int64(v) => *v.get(i)?,
            ColumnData::UInt64(v) => i64::try_from(*v.get(i)?).ok()?,
            _ => return None,
        })
    }

    /// Row `i` as text, for UTF-8 columns.
    pub fn get_str(&self, i: usize) -> Option<&str> {
        match self {
            ColumnData::Utf8(v) if i < v.len() => std::str::from_utf8(v.get(i)).ok(),
            _ => None,
        }
    }

    /// The whole column as `f64` (numeric scalar columns only).
    pub fn to_f64_vec(&self) -> Option<Vec<f64>> {
        macro_rules! conv {
            ($v:expr) => {
                Some($v.iter().map(|x| *x as f64).collect())
            };
        }
        match self {
            ColumnData::Int8(v) => conv!(v),
            ColumnData::UInt8(v) => conv!(v),
            ColumnData::Int16(v) => conv!(v),
            ColumnData::UInt16(v) => conv!(v),
            ColumnData::Int32(v) => conv!(v),
            ColumnData::UInt32(v) => conv!(v),
            ColumnData::Int64(v) => conv!(v),
            ColumnData::UInt64(v) => conv!(v),
            ColumnData::Float32(v) => conv!(v),
            ColumnData::Float64(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Keeps only the rows whose index is in `rows` (ascending).
    pub fn take(&self, rows: &[usize]) -> ColumnData {
        macro_rules! pick {
            ($variant:ident, $v:expr) => {
                ColumnData::$variant(rows.iter().map(|&r| $v[r].clone()).collect())
            };
        }
        match self {
            ColumnData::Int8(v) => pick!(Int8, v),
            ColumnData::UInt8(v) => pick!(UInt8, v),
            ColumnData::Int16(v) => pick!(Int16, v),
            ColumnData::UInt16(v) => pick!(UInt16, v),
            ColumnData::Int32(v) => pick!(Int32, v),
            ColumnData::UInt32(v) => pick!(UInt32, v),
            ColumnData::Int64(v) => pick!(Int64, v),
            ColumnData::UInt64(v) => pick!(UInt64, v),
            ColumnData::Float32(v) => pick!(Float32, v),
            ColumnData::Float64(v) => pick!(Float64, v),
            ColumnData::Utf8(v) | ColumnData::Binary(v) => {
                let mut out = VarData::new();
                for &r in rows {
                    out.push(v.get(r));
                }
                if matches!(self, ColumnData::Utf8(_)) {
                    ColumnData::Utf8(out)
                } else {
                    ColumnData::Binary(out)
                }
            }
            ColumnData::FixedList { size, values } => {
                let flat: Vec<usize> = rows
                    .iter()
                    .flat_map(|&r| (r * size)..(r * size + size))
                    .collect();
                ColumnData::FixedList {
                    size: *size,
                    values: Box::new(values.take(&flat)),
                }
            }
        }
    }

    pub(crate) fn shrink(&mut self) {
        match self {
            ColumnData::Int8(v) => v.shrink_to_fit(),
            ColumnData::UInt8(v) => v.shrink_to_fit(),
            ColumnData::Int16(v) => v.shrink_to_fit(),
            ColumnData::UInt16(v) => v.shrink_to_fit(),
            ColumnData::Int32(v) => v.shrink_to_fit(),
            ColumnData::UInt32(v) => v.shrink_to_fit(),
            ColumnData::Int64(v) => v.shrink_to_fit(),
            ColumnData::UInt64(v) => v.shrink_to_fit(),
            ColumnData::Float32(v) => v.shrink_to_fit(),
            ColumnData::Float64(v) => v.shrink_to_fit(),
            ColumnData::Utf8(v) | ColumnData::Binary(v) => {
                v.data.shrink_to_fit();
                v.offsets.shrink_to_fit();
            }
            ColumnData::FixedList { values, .. } => values.shrink(),
        }
    }
}

/// One named column.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Field name as written by the logger.
    pub name: String,
    /// Unit, when the log (FMTU/UNIT/MULT) or the MAVLink definition gives
    /// one. For DataFlash this is the unit pymavlink shows: the UNIT label
    /// with the MULT prefix folded in (`"cm"`, `"m/s/s"`, `"1e-07 deglatitude"`).
    pub unit: Option<String>,
    /// DataFlash MULT multiplier of the raw value, when the log gives one
    /// (informational: values are not multiplied, same as pymavlink).
    pub multiplier: Option<f64>,
    /// The values.
    pub data: ColumnData,
}

/// All messages of one type.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Message name (`"ATT"`, `"GPS_RAW_INT"`, ...).
    pub name: String,
    /// The columns, in the order the message defines its fields.
    pub columns: Vec<Column>,
    /// Field that distinguishes instances (`"I"`, `"C"`, ...), when the log marks one.
    pub instance_field: Option<String>,
    /// DataFlash format string (`"QBffff"`), when read from a .bin log.
    pub format: Option<String>,
    /// Number of rows (also correct for a message type without fields).
    pub rows: usize,
}

impl Table {
    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// The column with this name.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Column values as `f64` (numeric scalar columns only).
    pub fn f64s(&self, name: &str) -> Option<Vec<f64>> {
        self.column(name)?.data.to_f64_vec()
    }

    /// Column names, in order.
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }

    /// The rows of one instance (all rows when the table has no instance field).
    pub fn instance(&self, value: i64) -> Table {
        let Some(field) = &self.instance_field else {
            return self.clone();
        };
        let Some(col) = self.column(field) else {
            return self.clone();
        };
        let rows: Vec<usize> = (0..self.len())
            .filter(|&i| col.data.get_i64(i) == Some(value))
            .collect();
        self.take(&rows)
    }

    /// Distinct instance values in order of first appearance.
    pub fn instances(&self) -> Vec<i64> {
        let mut out = Vec::new();
        if let Some(col) = self.instance_field.as_deref().and_then(|f| self.column(f)) {
            for i in 0..col.data.len() {
                if let Some(v) = col.data.get_i64(i) {
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        out
    }

    /// A table with only the given rows (ascending indices).
    pub fn take(&self, rows: &[usize]) -> Table {
        Table {
            name: self.name.clone(),
            columns: self
                .columns
                .iter()
                .map(|c| Column {
                    name: c.name.clone(),
                    unit: c.unit.clone(),
                    multiplier: c.multiplier,
                    data: c.data.take(rows),
                })
                .collect(),
            instance_field: self.instance_field.clone(),
            format: self.format.clone(),
            rows: rows.len(),
        }
    }
}

//! Arrow conversion (feature `arrow`).
//!
//! Each [`Table`] becomes a `RecordBatch`. Column units and DataFlash
//! multipliers go into field metadata (`unit`, `multiplier`); the message
//! name, instance field and DataFlash format string go into schema metadata
//! (`flightlog.message`, `flightlog.instance_field`, `flightlog.format`).

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, FixedSizeListArray, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, LargeBinaryArray, LargeStringArray, RecordBatch, StringArray,
    UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::{Buffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType as ADT, Field, Schema};

use crate::column::{Column, ColumnData, Table, VarData};

fn var_offsets_i32(v: &VarData) -> Option<OffsetBuffer<i32>> {
    let offs: Option<Vec<i32>> = v.offsets.iter().map(|&o| i32::try_from(o).ok()).collect();
    offs.map(|o| OffsetBuffer::new(ScalarBuffer::from(o)))
}

fn var_offsets_i64(v: &VarData) -> OffsetBuffer<i64> {
    OffsetBuffer::new(ScalarBuffer::from(
        v.offsets.iter().map(|&o| o as i64).collect::<Vec<_>>(),
    ))
}

/// Arrow array and type of one column (consumes the data).
pub fn column_to_array(data: ColumnData) -> Result<(ArrayRef, ADT), ArrowError> {
    Ok(match data {
        ColumnData::Int8(v) => (Arc::new(Int8Array::from(v)), ADT::Int8),
        ColumnData::UInt8(v) => (Arc::new(UInt8Array::from(v)), ADT::UInt8),
        ColumnData::Int16(v) => (Arc::new(Int16Array::from(v)), ADT::Int16),
        ColumnData::UInt16(v) => (Arc::new(UInt16Array::from(v)), ADT::UInt16),
        ColumnData::Int32(v) => (Arc::new(Int32Array::from(v)), ADT::Int32),
        ColumnData::UInt32(v) => (Arc::new(UInt32Array::from(v)), ADT::UInt32),
        ColumnData::Int64(v) => (Arc::new(Int64Array::from(v)), ADT::Int64),
        ColumnData::UInt64(v) => (Arc::new(UInt64Array::from(v)), ADT::UInt64),
        ColumnData::Float32(v) => (Arc::new(Float32Array::from(v)), ADT::Float32),
        ColumnData::Float64(v) => (Arc::new(Float64Array::from(v)), ADT::Float64),
        ColumnData::Utf8(v) => match var_offsets_i32(&v) {
            Some(o) => (
                Arc::new(StringArray::try_new(o, Buffer::from_vec(v.data), None)?),
                ADT::Utf8,
            ),
            None => {
                let o = var_offsets_i64(&v);
                (
                    Arc::new(LargeStringArray::try_new(
                        o,
                        Buffer::from_vec(v.data),
                        None,
                    )?),
                    ADT::LargeUtf8,
                )
            }
        },
        ColumnData::Binary(v) => match var_offsets_i32(&v) {
            Some(o) => (
                Arc::new(BinaryArray::try_new(o, Buffer::from_vec(v.data), None)?),
                ADT::Binary,
            ),
            None => {
                let o = var_offsets_i64(&v);
                (
                    Arc::new(LargeBinaryArray::try_new(
                        o,
                        Buffer::from_vec(v.data),
                        None,
                    )?),
                    ADT::LargeBinary,
                )
            }
        },
        ColumnData::FixedList { size, values } => {
            let (inner, inner_ty) = column_to_array(*values)?;
            let item = Arc::new(Field::new("item", inner_ty, false));
            let size = i32::try_from(size)
                .map_err(|_| ArrowError::InvalidArgumentError("list too long".into()))?;
            let arr = FixedSizeListArray::try_new(item.clone(), size, inner, None)?;
            (Arc::new(arr), ADT::FixedSizeList(item, size))
        }
    })
}

fn field_for(c: &Column, ty: ADT) -> Field {
    let mut md = HashMap::new();
    if let Some(u) = &c.unit {
        md.insert("unit".to_string(), u.clone());
    }
    if let Some(m) = c.multiplier {
        md.insert("multiplier".to_string(), m.to_string());
    }
    Field::new(&c.name, ty, false).with_metadata(md)
}

impl Table {
    /// Converts to an Arrow `RecordBatch`, consuming the table.
    pub fn into_record_batch(self) -> Result<RecordBatch, ArrowError> {
        let mut md = HashMap::new();
        md.insert("flightlog.message".to_string(), self.name.clone());
        if let Some(i) = &self.instance_field {
            md.insert("flightlog.instance_field".to_string(), i.clone());
        }
        if let Some(f) = &self.format {
            md.insert("flightlog.format".to_string(), f.clone());
        }
        let rows = self.len();
        let mut fields = Vec::with_capacity(self.columns.len());
        let mut arrays = Vec::with_capacity(self.columns.len());
        for mut c in self.columns {
            let data = std::mem::replace(&mut c.data, ColumnData::UInt8(Vec::new()));
            let (arr, ty) = column_to_array(data)?;
            fields.push(field_for(&c, ty));
            arrays.push(arr);
        }
        let schema = Arc::new(Schema::new_with_metadata(fields, md));
        RecordBatch::try_new_with_options(
            schema,
            arrays,
            &arrow_array::RecordBatchOptions::new().with_row_count(Some(rows)),
        )
    }

    /// Converts to an Arrow `RecordBatch` (copies the data).
    pub fn to_record_batch(&self) -> Result<RecordBatch, ArrowError> {
        self.clone().into_record_batch()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Log, ReadOptions};

    #[test]
    fn tlog_tables_convert() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mini.tlog");
        let log = Log::open(path, &ReadOptions::default()).unwrap();
        for t in log.tables() {
            let b = t.to_record_batch().unwrap();
            assert_eq!(b.num_rows(), t.len());
            assert_eq!(b.num_columns(), t.columns.len());
        }
    }
}

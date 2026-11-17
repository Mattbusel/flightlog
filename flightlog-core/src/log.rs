//! [`Log`]: every message type of one flight log as a [`Table`].

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::column::Table;

/// Which container the log came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// ArduPilot DataFlash `.bin` (onboard log).
    DataFlash,
    /// MAVLink `.tlog` (ground station telemetry log).
    Tlog,
}

impl LogKind {
    /// `"dataflash"` or `"tlog"`.
    pub fn name(self) -> &'static str {
        match self {
            LogKind::DataFlash => "dataflash",
            LogKind::Tlog => "tlog",
        }
    }

    /// Guesses the container from the first bytes.
    ///
    /// DataFlash logs start with the `0xA3 0x95` header of a FMT message;
    /// tlogs start with an 8 byte timestamp followed by a MAVLink magic byte.
    /// Returns `None` when neither matches.
    pub fn detect(bytes: &[u8]) -> Option<LogKind> {
        if bytes.len() >= 3 && bytes[0] == 0xA3 && bytes[1] == 0x95 {
            return Some(LogKind::DataFlash);
        }
        if bytes.len() > 8 && (bytes[8] == 0xFE || bytes[8] == 0xFD) {
            return Some(LogKind::Tlog);
        }
        // Damaged start: count DataFlash headers in the first 64 KiB.
        let head = &bytes[..bytes.len().min(65536)];
        let df = head.windows(2).filter(|w| w == &[0xA3, 0x95]).count();
        if df >= 4 {
            return Some(LogKind::DataFlash);
        }
        let mut probe = mavlink_tlog::TlogSlice::new(head);
        if probe.by_ref().take(4).count() >= 2 {
            return Some(LogKind::Tlog);
        }
        None
    }
}

/// Options for reading a log.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    /// Only build tables for these message names (all when `None`).
    /// Messages of other types are still counted in [`Log::counts`], and
    /// FMT/FMTU/UNIT/MULT/PARM (or PARAM_VALUE) are always interpreted.
    pub types: Option<HashSet<String>>,
    /// Apply the DataFlash format multipliers (`c`, `C`, `e`, `E` divide by
    /// 100, `L` by 1e7) like pymavlink's DFReader does. Default `true`.
    /// When `false` those columns keep their raw integers.
    pub scale: bool,
    /// Record the order of all messages ([`Log::order`]), for replaying the
    /// log message by message. Default `false`.
    pub keep_order: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        ReadOptions {
            types: None,
            scale: true,
            keep_order: false,
        }
    }
}

impl ReadOptions {
    pub(crate) fn wants(&self, name: &str) -> bool {
        self.types.as_ref().is_none_or(|t| t.contains(name))
    }
}

/// A run of bytes the reader could not use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedRegion {
    /// Byte offset of the first skipped byte.
    pub offset: u64,
    /// Number of bytes skipped.
    pub len: u64,
}

/// How much of the file the reader could use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Integrity {
    /// File size.
    pub bytes: u64,
    /// Messages decoded.
    pub messages: u64,
    /// Bytes skipped while resynchronizing (damage or garbage between messages).
    pub skipped_bytes: u64,
    /// The first [`MAX_REGIONS`] skipped runs.
    pub skipped_regions: Vec<SkippedRegion>,
    /// Total number of skipped runs (may exceed `skipped_regions.len()`).
    pub skipped_region_count: u64,
    /// Bytes at the end that did not form a complete message.
    pub truncated_tail_bytes: u64,
    /// tlog: frames whose checksum was wrong.
    pub bad_crc: u64,
    /// Messages whose type is not known (tlog: id not in the dialect;
    /// DataFlash: type whose FMT could not be used), by id.
    pub unknown_types: BTreeMap<u32, u64>,
    /// DataFlash FMT definitions that were rejected, with the reason.
    pub invalid_formats: Vec<String>,
}

/// Cap on [`Integrity::skipped_regions`].
pub const MAX_REGIONS: usize = 256;

impl Integrity {
    pub(crate) fn skip(&mut self, offset: u64, len: u64) {
        if len == 0 {
            return;
        }
        self.skipped_bytes += len;
        self.skipped_region_count += 1;
        if self.skipped_regions.len() < MAX_REGIONS {
            self.skipped_regions.push(SkippedRegion { offset, len });
        }
    }

    /// True when nothing was skipped or cut off.
    pub fn is_clean(&self) -> bool {
        self.skipped_bytes == 0 && self.truncated_tail_bytes == 0 && self.bad_crc == 0
    }
}

/// Every message type of one log as a table.
#[derive(Debug, Clone)]
pub struct Log {
    /// The container.
    pub kind: LogKind,
    pub(crate) tables: Vec<Table>,
    pub(crate) index: HashMap<String, usize>,
    /// Messages per type name, including types not selected by
    /// [`ReadOptions::types`].
    pub counts: BTreeMap<String, u64>,
    /// Parameters (last value seen). DataFlash: PARM; tlog: PARAM_VALUE.
    pub params: BTreeMap<String, f64>,
    /// What the reader had to skip.
    pub integrity: Integrity,
    /// DataFlash: Unix time in microseconds at `TimeUS == 0`, from the first
    /// GPS message with a week number (same rule as pymavlink). `None` when
    /// the log has no GPS time. For a tlog it is `Some(0)`: its timestamps
    /// are already Unix microseconds.
    pub utc_offset_us: Option<i64>,
    /// With [`ReadOptions::keep_order`]: `(table index, row)` of every
    /// message in file order.
    pub order: Option<Vec<(u32, u32)>>,
}

impl Log {
    /// Reads a log, detecting the container from its content.
    pub fn read(bytes: &[u8], opts: &ReadOptions) -> Result<Log, crate::Error> {
        match LogKind::detect(bytes) {
            Some(LogKind::DataFlash) => Ok(crate::dataflash::read(bytes, opts)),
            Some(LogKind::Tlog) => Ok(crate::tlog::read(bytes, opts)),
            None => Err(crate::Error::UnknownFormat),
        }
    }

    /// Reads a log file from disk.
    pub fn open(
        path: impl AsRef<std::path::Path>,
        opts: &ReadOptions,
    ) -> Result<Log, crate::Error> {
        let bytes = std::fs::read(path)?;
        Log::read(&bytes, opts)
    }

    /// The table of one message type.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.index.get(name).map(|&i| &self.tables[i])
    }

    /// All tables, in order of first appearance.
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// Message type names that have a table, in order of first appearance.
    pub fn names(&self) -> Vec<&str> {
        self.tables.iter().map(|t| t.name.as_str()).collect()
    }

    /// Parameter value.
    pub fn param(&self, name: &str) -> Option<f64> {
        self.params.get(name).copied()
    }

    /// Name of the time column: `"TimeUS"` (DataFlash) or `"timestamp_us"` (tlog).
    pub fn time_column(&self) -> &'static str {
        match self.kind {
            LogKind::DataFlash => "TimeUS",
            LogKind::Tlog => "timestamp_us",
        }
    }

    /// First and last timestamp over all tables, in the log's time base.
    pub fn time_range_us(&self) -> Option<(u64, u64)> {
        let col = self.time_column();
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        for t in &self.tables {
            if let Some(c) = t.column(col) {
                for i in 0..c.data.len() {
                    if let Some(v) = c.data.get_f64(i) {
                        // Some messages log TimeUS 0 before the clock starts.
                        if v <= 0.0 {
                            continue;
                        }
                        let v = v as u64;
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
        }
        (lo <= hi).then_some((lo, hi))
    }

    pub(crate) fn from_parts(
        kind: LogKind,
        tables: Vec<Table>,
        counts: BTreeMap<String, u64>,
        params: BTreeMap<String, f64>,
        integrity: Integrity,
        utc_offset_us: Option<i64>,
        order: Option<Vec<(u32, u32)>>,
    ) -> Log {
        let index = tables
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        Log {
            kind,
            tables,
            index,
            counts,
            params,
            integrity,
            utc_offset_us,
            order,
        }
    }
}

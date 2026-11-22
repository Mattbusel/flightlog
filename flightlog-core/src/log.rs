//! [`Log`]: every message type of one flight log as a [`Table`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

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
    /// A tlog starts with an 8 byte timestamp followed by a MAVLink frame;
    /// it is recognized when that first frame's checksum verifies. A
    /// DataFlash log starts with `0xA3 0x95 0x80`, the header of a FMT
    /// message. When neither matches (a damaged start), the first 64 KiB
    /// are searched for DataFlash headers and for tlog records. Returns
    /// `None` when nothing matches.
    pub fn detect(bytes: &[u8]) -> Option<LogKind> {
        if bytes.len() > 8
            && (bytes[8] == 0xFE || bytes[8] == 0xFD)
            && mavlink_tlog::FrameRef::parse(&bytes[8..], mavlink_tlog::CrcPolicy::Strict).is_ok()
        {
            return Some(LogKind::Tlog);
        }
        if bytes.len() >= 3 && bytes[0] == 0xA3 && bytes[1] == 0x95 && bytes[2] == 0x80 {
            return Some(LogKind::DataFlash);
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

    /// True when every message up to the end was decoded: nothing skipped,
    /// no bad tlog checksum and no rejected DataFlash format.
    ///
    /// A few bytes cut off at the very end
    /// ([`truncated_tail_bytes`](Self::truncated_tail_bytes)) do not count:
    /// the logger stopping in the middle of its last message is normal.
    /// flight-facts `integrity.clean` uses this same definition.
    pub fn is_clean(&self) -> bool {
        self.skipped_bytes == 0 && self.bad_crc == 0 && self.invalid_formats.is_empty()
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
    /// Time range and outlier count, computed on first use.
    time: OnceLock<TimeSpan>,
}

/// Tolerance of the time range rules of [`Log::time_range_us`]: a timestamp
/// needs two other timestamps of the log within this many microseconds, and
/// may not step back by more than this from the previous row of its own
/// message type (or ahead of the next row by more than this).
///
/// Logs record several messages per second and their time only moves
/// forward, so real timestamps meet both rules. A timestamp damaged in the
/// file (for example a flipped bit in `TimeUS`) breaks at least one.
pub const TIME_NEIGHBOUR_US: u64 = 2_000_000;

#[derive(Debug, Clone, Copy)]
struct TimeSpan {
    range: Option<(u64, u64)>,
    outliers: u64,
}

impl Log {
    /// Reads a log, detecting the container from its content.
    ///
    /// Returns [`Error::UnknownFormat`](crate::Error::UnknownFormat) when the
    /// container cannot be detected or when not a single message decodes.
    pub fn read(bytes: &[u8], opts: &ReadOptions) -> Result<Log, crate::Error> {
        let log = match LogKind::detect(bytes) {
            Some(LogKind::DataFlash) => crate::dataflash::read(bytes, opts),
            Some(LogKind::Tlog) => crate::tlog::read(bytes, opts),
            None => return Err(crate::Error::UnknownFormat),
        };
        if log.integrity.messages == 0 {
            return Err(crate::Error::UnknownFormat);
        }
        Ok(log)
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

    /// First and last timestamp of the log, in the log's time base.
    ///
    /// The range runs from the first to the last timestamp that has two
    /// other timestamps within [`TIME_NEIGHBOUR_US`] and is in order with
    /// the rows before and after it in its own table (within the same
    /// tolerance). One damaged `TimeUS` therefore cannot stretch the range;
    /// [`time_outliers`](Self::time_outliers) counts the rows outside it.
    /// When no timestamp meets the rules (a log of a few sparse messages),
    /// the plain minimum and maximum are used. Zero timestamps (messages
    /// logged before the clock starts) are ignored.
    pub fn time_range_us(&self) -> Option<(u64, u64)> {
        self.time_span().range
    }

    /// Rows whose timestamp lies outside [`time_range_us`](Self::time_range_us):
    /// isolated timestamps, normally from damage in the file.
    pub fn time_outliers(&self) -> u64 {
        self.time_span().outliers
    }

    fn time_span(&self) -> TimeSpan {
        *self
            .time
            .get_or_init(|| tables_time_span(&self.tables, self.time_column()))
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
            time: OnceLock::new(),
        }
    }
}

fn tables_time_span(tables: &[Table], col: &str) -> TimeSpan {
    let mut ts: Vec<(u64, bool)> = Vec::new();
    for t in tables {
        if let Some(c) = t.column(col) {
            // Some messages log TimeUS 0 before the clock starts.
            let v: Vec<u64> = (0..c.data.len())
                .filter_map(|i| c.data.get_f64(i))
                .filter(|v| *v > 0.0 && v.is_finite())
                .map(|v| v as u64)
                .collect();
            ts.extend(in_order(&v));
        }
    }
    time_span(ts)
}

/// [`Log::time_range_us`] over any set of tables with a time column.
pub(crate) fn table_time_range(tables: &[Table], col: &str) -> Option<(u64, u64)> {
    tables_time_span(tables, col).range
}

/// Each timestamp of one table with whether it is in order with its
/// neighbours in that table: not more than [`TIME_NEIGHBOUR_US`] before the
/// previous row or after the next row.
fn in_order(v: &[u64]) -> impl Iterator<Item = (u64, bool)> + '_ {
    let w = TIME_NEIGHBOUR_US;
    (0..v.len()).map(move |i| {
        let after_prev = i == 0 || v[i].saturating_add(w) >= v[i - 1];
        let before_next = i + 1 == v.len() || v[i] <= v[i + 1].saturating_add(w);
        (v[i], after_prev && before_next)
    })
}

fn time_span(mut ts: Vec<(u64, bool)>) -> TimeSpan {
    if ts.is_empty() {
        return TimeSpan {
            range: None,
            outliers: 0,
        };
    }
    ts.sort_unstable();
    let n = ts.len();
    let w = TIME_NEIGHBOUR_US;
    // in order in its table, and at least two other timestamps within w
    let good = |i: usize| -> bool {
        ts[i].1
            && ((i + 2 < n && ts[i + 2].0 - ts[i].0 <= w)
                || (i >= 2 && ts[i].0 - ts[i - 2].0 <= w)
                || (i >= 1
                    && i + 1 < n
                    && ts[i].0 - ts[i - 1].0 <= w
                    && ts[i + 1].0 - ts[i].0 <= w))
    };
    match (0..n).find(|&i| good(i)) {
        Some(lo) => {
            let hi = (lo..n).rev().find(|&i| good(i)).unwrap_or(lo);
            let (lo, hi) = (ts[lo].0, ts[hi].0);
            TimeSpan {
                range: Some((lo, hi)),
                outliers: ts.iter().filter(|t| t.0 < lo || t.0 > hi).count() as u64,
            }
        }
        None => TimeSpan {
            range: Some((ts[0].0, ts[n - 1].0)),
            outliers: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(tables: &[Vec<u64>]) -> TimeSpan {
        time_span(tables.iter().flat_map(|t| in_order(t)).collect())
    }

    #[test]
    fn time_span_drops_damaged_timestamps() {
        let mut a: Vec<u64> = (0..100).map(|i| 3_000_000 + i * 100_000).collect();
        // far in the future, between two good rows
        a.insert(50, 1 << 62);
        // near zero, as the last row (2 s after a good row would still count)
        a.push(5);
        let b: Vec<u64> = (0..100).map(|i| 3_050_000 + i * 100_000).collect();
        let s = span(&[a, b]);
        assert_eq!(s.range, Some((3_000_000, 12_950_000)));
        assert_eq!(s.outliers, 2);
    }

    #[test]
    fn time_span_drops_isolated_timestamps() {
        // a table of its own with one far timestamp
        let a: Vec<u64> = (0..100).map(|i| 3_000_000 + i * 100_000).collect();
        let s = span(&[a, vec![1 << 62]]);
        assert_eq!(s.range, Some((3_000_000, 12_900_000)));
        assert_eq!(s.outliers, 1);
    }

    #[test]
    fn time_span_keeps_sparse_logs() {
        let s = span(&[vec![10_000_000, 40_000_000, 90_000_000]]);
        assert_eq!(s.range, Some((10_000_000, 90_000_000)));
        assert_eq!(s.outliers, 0);
        assert_eq!(span(&[]).range, None);
    }

    #[test]
    fn time_span_keeps_logging_pauses() {
        // logging stopped for 60 s in every table
        let a: Vec<u64> = (0..50)
            .map(|i| 1_000_000 + i * 100_000)
            .chain((0..50).map(|i| 70_000_000 + i * 100_000))
            .collect();
        let b: Vec<u64> = a.iter().map(|t| t + 20_000).collect();
        let s = span(&[a, b]);
        assert_eq!(s.range, Some((1_000_000, 74_920_000)));
        assert_eq!(s.outliers, 0);
    }

    #[test]
    fn detect_needs_a_verified_tlog_frame_or_fmt_header() {
        // a MAVLink magic byte at offset 8 alone does not make a tlog
        assert_eq!(LogKind::detect(b"hello wo\xfdrld, not a log at all"), None);
        assert_eq!(LogKind::detect(b"\xa3\x95\x80"), Some(LogKind::DataFlash));
        // DataFlash header bytes without the FMT type are not enough either
        assert_eq!(LogKind::detect(b"\xa3\x95\x01 plain text"), None);
    }
}

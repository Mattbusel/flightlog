//! flight-facts: deterministic checks over a flight log.
//!
//! Every check uses a threshold the ArduPilot wiki documents (or the
//! vehicle's own parameter that the wiki names), reports the wiki page and
//! the verbatim sentence it rests on, the measured values, and the time
//! ranges involved. Nothing here is a model or a guess: the same log always
//! gives the same document. The output follows the flight-facts v1 JSON
//! Schema ([`SCHEMA`], `schemas/flight-facts-v1.schema.json`).
//!
//! ```no_run
//! use flightlog_core::{facts, Log, ReadOptions};
//! let log = Log::open("00000001.BIN", &ReadOptions::default())?;
//! let doc = facts::run(&log, &facts::Options::default());
//! for c in &doc.checks {
//!     println!("{:<22} {:?} {}", c.id, c.status, c.summary);
//! }
//! # Ok::<(), flightlog_core::Error>(())
//! ```

mod checks;
mod series;
pub mod wiki;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{Log, LogKind};

/// The flight-facts v1 JSON Schema.
pub const SCHEMA: &str = include_str!("../../schemas/flight-facts-v1.schema.json");

/// `$id` of the schema.
pub const SCHEMA_ID: &str = "https://raw.githubusercontent.com/Mattbusel/flightlog/main/flightlog-core/schemas/flight-facts-v1.schema.json";

/// Version of the flight-facts format.
pub const VERSION: &str = "1.0";

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Percentile used to read the wiki's "mostly below 30m/s/s" for
    /// vibration. Default 0.95.
    pub vibe_percentile: f64,
    /// Include the event list (ERR, arming, mode changes). Default true.
    pub events: bool,
    /// Gaps shorter than this join two threshold crossings into one range.
    /// Default 1 s.
    pub merge_gap_us: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            vibe_percentile: 0.95,
            events: true,
            merge_gap_us: 1_000_000,
        }
    }
}

/// Result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Within the wiki's thresholds.
    Pass,
    /// Worth knowing, but within what the wiki calls acceptable.
    Info,
    /// The wiki says this may be a problem.
    Warn,
    /// The wiki says this is a problem, or the firmware logged the failsafe.
    Fail,
    /// The log does not contain the data this check needs.
    NotApplicable,
}

/// Where a threshold value came from.
#[derive(Debug, Clone, Serialize)]
pub struct Threshold {
    /// What the value limits (`"warn"`, `"fail"`, `"BATT_LOW_VOLT"`, ...).
    pub name: String,
    /// The value.
    pub value: f64,
    /// Unit.
    pub unit: String,
    /// `"wiki"`, `"param"` (read from the log) or `"param_default"` (the
    /// default the wiki states, because the log has no value).
    pub origin: String,
}

/// A wiki citation.
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    /// Published page.
    pub url: String,
    /// Verbatim sentence from the page source.
    pub quote: String,
    /// File in ArduPilot/ardupilot_wiki.
    pub wiki_file: String,
    /// ardupilot_wiki commit the quote was checked at.
    pub wiki_commit: String,
}

impl From<wiki::Cite> for Source {
    fn from(c: wiki::Cite) -> Self {
        Source {
            url: c.url.into(),
            quote: c.quote.into(),
            wiki_file: c.file.into(),
            wiki_commit: wiki::WIKI_COMMIT.into(),
        }
    }
}

/// A time range a check points at.
#[derive(Debug, Clone, Serialize)]
pub struct Range {
    /// Start, log time base (DataFlash TimeUS or tlog Unix microseconds).
    pub start_us: u64,
    /// End, same time base.
    pub end_us: u64,
    /// Start in seconds from the first message of the log.
    pub start_s: f64,
    /// End in seconds from the first message of the log.
    pub end_s: f64,
    /// What happens in this range.
    pub detail: String,
}

/// One check.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// Stable id, for example `"vibration.levels"`.
    pub id: String,
    /// Result.
    pub status: Status,
    /// One sentence a person can read.
    pub summary: String,
    /// Thresholds used.
    pub thresholds: Vec<Threshold>,
    /// Measured values (named numbers or small objects).
    pub measured: Map<String, Value>,
    /// Time ranges that crossed a threshold or where the firmware logged the event.
    pub ranges: Vec<Range>,
    /// Fields read, as `MESSAGE.Field`.
    pub evidence: Vec<String>,
    /// Wiki statements the thresholds come from.
    pub sources: Vec<Source>,
}

/// A logged event.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Log time base.
    pub time_us: u64,
    /// Seconds from the first message.
    pub t_s: f64,
    /// `"error"`, `"armed"`, `"disarmed"`, `"mode"` or `"text"`.
    pub kind: String,
    /// Human-readable description.
    pub text: String,
    /// ERR subsystem, mode number, ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    /// ERR code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subcode: Option<i64>,
}

/// What the log is.
#[derive(Debug, Clone, Serialize)]
pub struct LogInfo {
    /// `"dataflash"` or `"tlog"`.
    pub kind: String,
    /// `"copter"`, `"plane"`, `"rover"`, `"sub"`, `"heli"`, `"blimp"`, `"tracker"` when known.
    pub vehicle: Option<String>,
    /// Firmware string, when logged.
    pub firmware: Option<String>,
    /// `"boot_us"` (DataFlash TimeUS) or `"unix_us"` (tlog).
    pub time_base: String,
    /// First timestamp.
    pub start_us: Option<u64>,
    /// Last timestamp.
    pub end_us: Option<u64>,
    /// Seconds between first and last timestamp.
    pub duration_s: f64,
    /// UTC time of the first message, when the log has GPS time.
    pub utc_start: Option<String>,
    /// Number of message types.
    pub message_types: usize,
    /// Number of messages.
    pub messages: u64,
}

/// How much of the file could be read.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrityInfo {
    /// File size.
    pub bytes: u64,
    /// Bytes skipped while resynchronizing.
    pub skipped_bytes: u64,
    /// Number of skipped runs.
    pub skipped_regions: u64,
    /// Bytes at the end that did not form a message.
    pub truncated_tail_bytes: u64,
    /// tlog frames with a wrong checksum.
    pub bad_crc: u64,
    /// Rejected DataFlash formats.
    pub invalid_formats: Vec<String>,
    /// True when nothing was skipped (a few bytes cut off at the very end are normal).
    pub clean: bool,
}

/// Armed time.
#[derive(Debug, Clone, Serialize)]
pub struct Armed {
    /// Where arming was read from (`"ARM.ArmState"`, `"EV.Id"`, `"HEARTBEAT.base_mode"` or `"none"`).
    pub source: String,
    /// Total armed seconds.
    pub total_s: f64,
    /// Armed ranges.
    pub ranges: Vec<Range>,
}

/// Counts of check results.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    /// `pass` checks.
    pub pass: usize,
    /// `info` checks.
    pub info: usize,
    /// `warn` checks.
    pub warn: usize,
    /// `fail` checks.
    pub fail: usize,
    /// `not_applicable` checks.
    pub not_applicable: usize,
}

/// A flight-facts document.
#[derive(Debug, Clone, Serialize)]
pub struct FlightFacts {
    /// Schema `$id`.
    #[serde(rename = "$schema")]
    pub schema: String,
    /// Format version (`"1.0"`).
    pub flight_facts_version: String,
    /// Program that made it.
    pub generator: Map<String, Value>,
    /// The log.
    pub log: LogInfo,
    /// Reader integrity.
    pub integrity: IntegrityInfo,
    /// Armed time.
    pub armed: Armed,
    /// Parameters the checks used, with their values in the log.
    pub params_used: Map<String, Value>,
    /// Check counts by status.
    pub summary: Summary,
    /// The checks.
    pub checks: Vec<Check>,
    /// Logged events (ERR, arming, mode changes, failsafe texts).
    pub events: Vec<Event>,
}

impl FlightFacts {
    /// Compact JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serializable")
    }

    /// Indented JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("serializable")
    }

    /// The check with this id.
    pub fn check(&self, id: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.id == id)
    }
}

/// Runs every check on a log.
pub fn run(log: &Log, opts: &Options) -> FlightFacts {
    let ctx = checks::Ctx::new(log, opts);
    let checks = checks::all(&ctx);
    let mut summary = Summary::default();
    for c in &checks {
        match c.status {
            Status::Pass => summary.pass += 1,
            Status::Info => summary.info += 1,
            Status::Warn => summary.warn += 1,
            Status::Fail => summary.fail += 1,
            Status::NotApplicable => summary.not_applicable += 1,
        }
    }
    let i = &log.integrity;
    let events = if opts.events {
        ctx.events()
    } else {
        Vec::new()
    };
    let params_used = ctx.params_used.borrow().clone();
    let mut generator = Map::new();
    generator.insert("name".into(), "flightlog".into());
    generator.insert("version".into(), env!("CARGO_PKG_VERSION").into());
    FlightFacts {
        schema: SCHEMA_ID.into(),
        flight_facts_version: VERSION.into(),
        generator,
        log: ctx.log_info(),
        integrity: IntegrityInfo {
            bytes: i.bytes,
            skipped_bytes: i.skipped_bytes,
            skipped_regions: i.skipped_region_count,
            truncated_tail_bytes: i.truncated_tail_bytes,
            bad_crc: i.bad_crc,
            invalid_formats: i.invalid_formats.clone(),
            clean: i.skipped_bytes == 0 && i.bad_crc == 0 && i.invalid_formats.is_empty(),
        },
        armed: Armed {
            source: ctx.armed_source.into(),
            total_s: checks::round3(series::total_s(&ctx.armed)),
            ranges: ctx
                .armed
                .iter()
                .map(|&(s, e)| ctx.range(s, e, "armed"))
                .collect(),
        },
        params_used,
        summary,
        checks,
        events,
    }
}

/// Unix microseconds to an ISO 8601 UTC string with millisecond precision.
pub fn iso8601(unix_us: i64) -> String {
    let secs = unix_us.div_euclid(1_000_000);
    let ms = unix_us.rem_euclid(1_000_000) / 1000;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // days to civil (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

pub(crate) fn kind_name(k: LogKind) -> &'static str {
    k.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_794_953_181_800_748), "2026-11-17T22:06:21.800Z");
        assert_eq!(iso8601(951_782_400_000_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn schema_parses() {
        let v: Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(v["$id"], SCHEMA_ID);
    }
}

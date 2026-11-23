//! Flight logs as tables.
//!
//! `flightlog-core` reads ArduPilot DataFlash logs (`.bin`, written on the
//! vehicle), MAVLink telemetry logs (`.tlog`, written by the ground
//! station) and PX4 ULog files (`.ulg`, [`ulog`]) into one typed, columnar
//! [`Table`] per message type. [`FlightLog`] opens any of them; [`track`]
//! exports the GPS track as GPX or KML, [`events`] lists mode changes,
//! arming, failsafes and warnings. For ArduPilot logs it also runs
//! **flight-facts**: deterministic checks whose thresholds come from the
//! ArduPilot wiki, each reported with the wiki link, the measured values and
//! the time ranges involved. [`report`] answers the operational questions
//! about one flight: how long, how high and how far it flew, why it ended
//! (clean landing, failsafe, lost recording, or crash signature), and whether
//! it exceeded a height ceiling such as the 400 ft FAA Part 107 limit.
//!
//! ```no_run
//! use flightlog_core::{Log, ReadOptions};
//!
//! let log = Log::open("00000001.BIN", &ReadOptions::default())?;
//! let att = log.table("ATT").expect("attitude logged");
//! let roll = att.f64s("Roll").unwrap();
//! println!("{} ATT rows, max roll {:.1}", att.len(), roll.iter().cloned().fold(f64::MIN, f64::max));
//! # #[cfg(feature = "facts")]
//! println!("{}", flightlog_core::facts::run(&log, &Default::default()).to_json_pretty());
//! # Ok::<(), flightlog_core::Error>(())
//! ```
//!
//! Values match pymavlink: DataFlash `c`/`C`/`e`/`E` fields are divided by
//! 100 and `L` (latitude/longitude) by 1e7, text is cut at the first NUL.
//! Damaged logs are read the way pymavlink's `DFReader` reads them, and
//! what had to be skipped is reported in [`Integrity`].
//!
//! Features: `facts` (default) adds [`facts`]; `arrow` adds
//! [`Table::to_record_batch`](crate::Table) conversions in [`arrow`](crate::arrow).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod column;
pub mod dataflash;
pub mod events;
mod flight;
mod log;
pub mod tlog;
pub mod track;
pub mod ulog;

#[cfg(feature = "arrow")]
pub mod arrow;
#[cfg(feature = "facts")]
pub mod facts;
#[cfg(feature = "facts")]
pub mod report;

pub use column::{Column, ColumnData, DataType, Table, VarData};
pub use flight::FlightLog;
pub use log::{Integrity, Log, LogKind, ReadOptions, SkippedRegion, MAX_REGIONS};

/// Errors from opening a log.
#[derive(Debug)]
pub enum Error {
    /// Reading the file failed.
    Io(std::io::Error),
    /// The bytes are neither a DataFlash log nor a tlog.
    UnknownFormat,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::UnknownFormat => write!(f, "not a DataFlash .bin or MAVLink .tlog log"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::UnknownFormat => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub(crate) fn dataflash_text(b: &[u8]) -> String {
    dataflash::decode_text_pub(b)
}

/// An `f32` as the `f64` with the same shortest decimal form (0.8f32 gives
/// 0.8, not 0.800000011920929), for parameters that are stored as floats.
pub fn short_f32(v: f32) -> f64 {
    if v.is_finite() {
        v.to_string().parse().unwrap_or(v as f64)
    } else {
        v as f64
    }
}

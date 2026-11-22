//! [`FlightLog`]: one API over ArduPilot (`.bin`, `.tlog`) and PX4 (`.ulg`) logs.

use std::collections::BTreeMap;

use crate::column::Table;
use crate::log::{Integrity, Log, ReadOptions};
use crate::track::TrackPoint;
use crate::ulog::{self, ULog};

/// A flight log of either autopilot, detected from its content.
///
/// ```no_run
/// use flightlog_core::{FlightLog, ReadOptions};
///
/// for path in ["00000001.BIN", "flight.tlog", "log_0.ulg"] {
///     let log = FlightLog::open(path, &ReadOptions::default())?;
///     let track = log.track();
///     println!("{path}: {} {} topics, {} GPS points, clean: {}",
///         log.format_name(), log.tables().len(), track.len(), log.integrity().is_clean());
/// }
/// # Ok::<(), flightlog_core::Error>(())
/// ```
#[derive(Debug, Clone)]
pub enum FlightLog {
    /// ArduPilot DataFlash `.bin` or MAVLink `.tlog`.
    ArduPilot(Log),
    /// PX4 ULog `.ulg`.
    Px4(ULog),
}

impl FlightLog {
    /// Reads a log from memory, detecting the format. ULog files are read
    /// in full; [`ReadOptions`] applies to DataFlash and tlog only.
    pub fn read(bytes: &[u8], opts: &ReadOptions) -> Result<FlightLog, crate::Error> {
        if ulog::is_ulog(bytes) {
            ULog::read(bytes).map(FlightLog::Px4)
        } else {
            Log::read(bytes, opts).map(FlightLog::ArduPilot)
        }
    }

    /// Reads a log file from disk.
    pub fn open(
        path: impl AsRef<std::path::Path>,
        opts: &ReadOptions,
    ) -> Result<FlightLog, crate::Error> {
        FlightLog::read(&std::fs::read(path)?, opts)
    }

    /// `"dataflash"`, `"tlog"` or `"ulog"`.
    pub fn format_name(&self) -> &'static str {
        match self {
            FlightLog::ArduPilot(l) => l.kind.name(),
            FlightLog::Px4(_) => "ulog",
        }
    }

    /// The table of one message type or topic.
    pub fn table(&self, name: &str) -> Option<&Table> {
        match self {
            FlightLog::ArduPilot(l) => l.table(name),
            FlightLog::Px4(l) => l.table(name),
        }
    }

    /// All tables.
    pub fn tables(&self) -> &[Table] {
        match self {
            FlightLog::ArduPilot(l) => l.tables(),
            FlightLog::Px4(l) => l.tables(),
        }
    }

    /// Name of the time column: `"TimeUS"`, `"timestamp_us"` or `"timestamp"`.
    pub fn time_column(&self) -> &'static str {
        match self {
            FlightLog::ArduPilot(l) => l.time_column(),
            FlightLog::Px4(_) => "timestamp",
        }
    }

    /// Parameters (last value in the log).
    pub fn params(&self) -> &BTreeMap<String, f64> {
        match self {
            FlightLog::ArduPilot(l) => &l.params,
            FlightLog::Px4(l) => &l.params,
        }
    }

    /// One parameter.
    pub fn param(&self, name: &str) -> Option<f64> {
        self.params().get(name).copied()
    }

    /// What the reader had to skip.
    pub fn integrity(&self) -> &Integrity {
        match self {
            FlightLog::ArduPilot(l) => &l.integrity,
            FlightLog::Px4(l) => &l.integrity,
        }
    }

    /// First and last timestamp, in the log's time base.
    pub fn time_range_us(&self) -> Option<(u64, u64)> {
        match self {
            FlightLog::ArduPilot(l) => l.time_range_us(),
            FlightLog::Px4(l) => l.time_range_us(),
        }
    }

    /// Unix microseconds at timestamp 0, when the log has GPS time.
    pub fn utc_offset_us(&self) -> Option<i64> {
        match self {
            FlightLog::ArduPilot(l) => l.utc_offset_us,
            FlightLog::Px4(l) => l.utc_offset_us(),
        }
    }

    /// The GPS track (see [`track`](crate::track)).
    pub fn track(&self) -> Vec<TrackPoint> {
        match self {
            FlightLog::ArduPilot(l) => crate::track::from_log(l),
            FlightLog::Px4(l) => crate::track::from_ulog(l),
        }
    }

    /// Mode changes, arming, failsafes and warnings (see [`events`](crate::events)).
    #[cfg(feature = "facts")]
    pub fn events(&self) -> Vec<crate::events::FlightEvent> {
        match self {
            FlightLog::ArduPilot(l) => crate::events::from_log(l),
            FlightLog::Px4(l) => crate::events::from_ulog(l),
        }
    }

    /// The ArduPilot log, if this is one.
    pub fn as_ardupilot(&self) -> Option<&Log> {
        match self {
            FlightLog::ArduPilot(l) => Some(l),
            FlightLog::Px4(_) => None,
        }
    }

    /// The PX4 log, if this is one.
    pub fn as_px4(&self) -> Option<&ULog> {
        match self {
            FlightLog::Px4(l) => Some(l),
            FlightLog::ArduPilot(_) => None,
        }
    }
}

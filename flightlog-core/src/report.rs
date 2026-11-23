//! flight-report: what happened on one flight, in the terms an investigator,
//! insurer or regulator asks about.
//!
//! [`FlightReport`] turns an ArduPilot (`.bin`, `.tlog`) or PX4 (`.ulg`) log
//! into three concrete answers:
//!
//! 1. a [`FlightSummary`]: how long it flew, how high above the takeoff
//!    point, how far from it, and how fast, each with the field it was read
//!    from;
//! 2. an [`Ending`]: why the flight ended, read from the log itself. A clean
//!    landing, a failsafe that forced the vehicle down (which failsafe), a
//!    recording that stopped while the vehicle was still airborne, or a crash
//!    signature. Each verdict names the evidence it rests on and whether the
//!    firmware reported it or flightlog inferred it;
//! 3. an [`AltitudeCompliance`] flag: did the flight exceed a height ceiling
//!    (by default 400 ft above the takeoff point, the FAA Part 107 limit),
//!    and in which time ranges.
//!
//! Every number comes from the log. Nothing is modelled or guessed, and the
//! same log always gives the same report. Height is measured above the
//! takeoff point (the autopilot's height-above-home or, for PX4, height above
//! the local origin), which equals height above ground over level terrain.
//!
//! ```no_run
//! use flightlog_core::{report, FlightLog, ReadOptions};
//!
//! let log = FlightLog::open("00000001.BIN", &ReadOptions::default())?;
//! let r = report::build(&log, &report::ReportOptions::default());
//! println!("{}", r.summary.headline());
//! println!("ended: {}", r.ending.summary);
//! if r.altitude.exceeded {
//!     println!("ceiling exceeded: {}", r.altitude.summary);
//! }
//! # Ok::<(), flightlog_core::Error>(())
//! ```

use serde::Serialize;

use crate::events::{EventKind, FlightEvent};
use crate::track::TrackPoint;
use crate::FlightLog;

/// 400 feet in metres, the FAA Part 107 height ceiling above ground level
/// (14 CFR 107.51(b)(1)).
pub const PART_107_CEILING_M: f64 = 121.92;

/// The reference the altitude ceiling check cites.
pub const PART_107_CITATION: &str =
    "14 CFR 107.51(b)(1): a small unmanned aircraft may not be operated higher \
     than 400 feet above ground level, unless within 400 feet of a structure.";

/// Options for [`build`].
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Height ceiling in metres above the takeoff point. Default
    /// [`PART_107_CEILING_M`] (400 ft).
    pub ceiling_agl_m: f64,
    /// A sustained descent faster than this (metres per second), ending while
    /// the vehicle is still armed, is reported as a possible crash signature.
    /// Default 8.0.
    pub crash_descent_ms: f64,
    /// Length of the window at the end of the flight that the crash signature
    /// looks at, in seconds. Default 3.0.
    pub end_window_s: f64,
    /// A flight that ends still armed and above this height (metres) is
    /// reported as having ended while airborne. Default 2.0.
    pub airborne_end_m: f64,
}

impl Default for ReportOptions {
    fn default() -> Self {
        ReportOptions {
            ceiling_agl_m: PART_107_CEILING_M,
            crash_descent_ms: 8.0,
            end_window_s: 3.0,
            airborne_end_m: 2.0,
        }
    }
}

/// A latitude/longitude point in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GeoPoint {
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
}

/// What a flight did, in plain terms.
#[derive(Debug, Clone, Serialize)]
pub struct FlightSummary {
    /// `"dataflash"`, `"tlog"` or `"ulog"`.
    pub format: String,
    /// Whole-log duration in seconds.
    pub log_duration_s: f64,
    /// Seconds the vehicle was armed, when arming is logged.
    pub armed_s: Option<f64>,
    /// True when the vehicle was armed at some point in the log.
    pub flew: bool,
    /// Greatest height above the takeoff point in metres.
    pub max_alt_agl_m: Option<f64>,
    /// Field the height came from (`"POS.RelHomeAlt"`,
    /// `"GLOBAL_POSITION_INT.relative_alt"`, `"vehicle_local_position.z"`, ...).
    pub alt_source: Option<String>,
    /// Greatest straight-line distance from the takeoff point in metres.
    pub max_distance_m: Option<f64>,
    /// Field the distance came from (`"GPS"` haversine, `"vehicle_local_position"`).
    pub distance_source: Option<String>,
    /// Greatest ground speed in metres per second.
    pub top_speed_ms: Option<f64>,
    /// Field the speed came from (`"GPS.Spd"`, `"VFR_HUD.groundspeed"`, ...).
    pub speed_source: Option<String>,
    /// First GPS fix (takeoff point), when the log has GPS.
    pub takeoff: Option<GeoPoint>,
    /// Last GPS fix (landing point), when the log has GPS.
    pub landing: Option<GeoPoint>,
    /// UTC time of the first message, when the log has GPS time.
    pub utc_start: Option<String>,
    /// Number of 3D GPS fixes in the log.
    pub gps_fixes: usize,
}

impl FlightSummary {
    /// One line a person can read.
    pub fn headline(&self) -> String {
        if !self.flew {
            return format!(
                "No armed flight in this {} log ({:.0} s recorded).",
                self.format, self.log_duration_s
            );
        }
        let dur = self.armed_s.unwrap_or(self.log_duration_s);
        let mut s = format!("Flew {:.0} s", dur);
        if let Some(a) = self.max_alt_agl_m {
            s.push_str(&format!(
                ", up to {:.0} m ({:.0} ft) above takeoff",
                a,
                a * 3.280_84
            ));
        }
        if let Some(d) = self.max_distance_m {
            s.push_str(&format!(", out to {:.0} m", d));
        }
        if let Some(v) = self.top_speed_ms {
            s.push_str(&format!(", top ground speed {:.1} m/s", v));
        }
        s.push('.');
        s
    }
}

/// Why the flight ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndCause {
    /// The vehicle landed and disarmed with no failsafe active and no crash
    /// signature: a normal end of flight.
    NormalLanding,
    /// The flight ended with a failsafe active; the vehicle was brought down
    /// by it (radio, battery, GCS link, ... in [`Ending::failsafe`]).
    FailsafeLanding,
    /// The log ended while the vehicle was still armed and above ground: the
    /// recording (or power) stopped in flight. The log may be truncated.
    EndedAirborne,
    /// A crash signature: the firmware logged a crash, PX4's failure detector
    /// tripped, or the height dropped faster than the configured rate while
    /// the vehicle stayed armed.
    CrashSignature,
    /// Arming is not logged, or the evidence does not point to one cause.
    Undetermined,
}

impl EndCause {
    /// Lower-case name.
    pub fn name(self) -> &'static str {
        match self {
            EndCause::NormalLanding => "normal_landing",
            EndCause::FailsafeLanding => "failsafe_landing",
            EndCause::EndedAirborne => "ended_airborne",
            EndCause::CrashSignature => "crash_signature",
            EndCause::Undetermined => "undetermined",
        }
    }
}

/// Whether the firmware reported the end cause or flightlog inferred it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// The log states it: a failsafe event, a crash message, or a disarm.
    Reported,
    /// Derived from the height and armed state, not stated by the firmware.
    Inferred,
}

/// One piece of evidence behind the end cause.
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    /// Seconds from the first message.
    pub t_s: f64,
    /// What it is.
    pub detail: String,
}

/// How and why the flight ended.
#[derive(Debug, Clone, Serialize)]
pub struct Ending {
    /// The cause.
    pub cause: EndCause,
    /// How sure the log lets us be.
    pub confidence: Confidence,
    /// One sentence a person can read.
    pub summary: String,
    /// The failsafe family when the cause is a failsafe (`"Radio Failsafe"`,
    /// `"Battery Failsafe"`, `"GCS Failsafe"`, `"failsafe"` for PX4, ...).
    pub failsafe: Option<String>,
    /// The flight mode at the end, when logged.
    pub final_mode: Option<String>,
    /// True when the vehicle was still armed when the log ended.
    pub ended_armed: bool,
    /// True when bytes at the end of the file did not form a message.
    pub truncated: bool,
    /// The events and signals the cause rests on.
    pub evidence: Vec<Evidence>,
}

/// Whether the flight stayed under the height ceiling.
#[derive(Debug, Clone, Serialize)]
pub struct AltitudeCompliance {
    /// The ceiling in metres above the takeoff point.
    pub ceiling_agl_m: f64,
    /// The ceiling in feet.
    pub ceiling_agl_ft: f64,
    /// True when the height exceeded the ceiling at any point.
    pub exceeded: bool,
    /// Greatest height above the takeoff point in metres, when known.
    pub max_alt_agl_m: Option<f64>,
    /// Field the height came from.
    pub alt_source: Option<String>,
    /// Time ranges above the ceiling: (start_s, end_s, peak metres).
    pub ranges: Vec<CeilingRange>,
    /// One sentence a person can read.
    pub summary: String,
    /// The rule the ceiling comes from.
    pub citation: String,
}

/// A time range above the ceiling.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CeilingRange {
    /// Start, seconds from the first message.
    pub start_s: f64,
    /// End, seconds from the first message.
    pub end_s: f64,
    /// Greatest height in the range, metres above takeoff.
    pub peak_m: f64,
}

/// A flight report.
#[derive(Debug, Clone, Serialize)]
pub struct FlightReport {
    /// Flight summary.
    pub summary: FlightSummary,
    /// Why the flight ended.
    pub ending: Ending,
    /// Height-ceiling compliance.
    pub altitude: AltitudeCompliance,
}

impl FlightReport {
    /// Compact JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serializable")
    }

    /// Indented JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("serializable")
    }
}

/// Height above the takeoff point over time, with the field it came from.
struct AltSeries {
    /// (timestamp in log time base, metres above takeoff).
    pts: Vec<(u64, f64)>,
    source: String,
}

/// Builds the report for a log.
pub fn build(log: &FlightLog, opts: &ReportOptions) -> FlightReport {
    let (start_us, end_us) = log.time_range_us().unwrap_or((0, 0));
    let events = log.events();
    let track = log.track();
    let alt = alt_series(log);
    let (armed, armed_s, ended_armed) = armed_from_events(&events, end_us);
    let flew = armed_s.is_some() && armed.iter().any(|&(s, e)| e > s);

    let summary = summary(log, &track, &alt, start_us, end_us, armed_s, flew);
    let ending = ending(
        log,
        &events,
        &alt,
        &armed,
        start_us,
        end_us,
        ended_armed,
        opts,
    );
    let altitude = altitude(&alt, start_us, opts);

    FlightReport {
        summary,
        ending,
        altitude,
    }
}

fn to_s(t: u64, start_us: u64) -> f64 {
    round3(t.saturating_sub(start_us) as f64 / 1e6)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Collapse a sub-metre height to 0.0 for display, so a landed vehicle prints
/// as `0 m` and never as `-0 m` (used only in the human-readable strings; the
/// numeric fields keep the measured value).
fn clean0(x: f64) -> f64 {
    if x.abs() < 0.5 {
        0.0
    } else {
        x
    }
}

fn round6(x: f64) -> f64 {
    if x.is_finite() && x.abs() < 1e9 {
        (x * 1e6).round() / 1e6
    } else {
        x
    }
}

/// Armed ranges from the event stream, total armed seconds, and whether the
/// log ended still armed (an `Armed` event with no later `Disarmed`).
fn armed_from_events(events: &[FlightEvent], end_us: u64) -> (Vec<(u64, u64)>, Option<f64>, bool) {
    let mut has_arm = false;
    let mut ranges = Vec::new();
    let mut since: Option<u64> = None;
    for e in events {
        match e.kind {
            EventKind::Armed => {
                has_arm = true;
                if since.is_none() {
                    since = Some(e.time_us);
                }
            }
            EventKind::Disarmed => {
                has_arm = true;
                if let Some(s) = since.take() {
                    ranges.push((s, e.time_us));
                }
            }
            _ => {}
        }
    }
    let ended_armed = since.is_some();
    if let Some(s) = since {
        ranges.push((s, end_us.max(s)));
    }
    if !has_arm {
        return (Vec::new(), None, false);
    }
    let total = ranges.iter().map(|&(s, e)| (e - s) as f64 / 1e6).sum();
    (ranges, Some(round3(total)), ended_armed)
}

/// Height above the takeoff point for each supported format.
fn alt_series(log: &FlightLog) -> Option<AltSeries> {
    match log {
        FlightLog::ArduPilot(l) => {
            let tcol = l.time_column();
            // POS.RelHomeAlt, then CTUN.Alt, then BARO.Alt: all heights above
            // the home/startup point in metres.
            for (table, field) in [
                ("POS", "RelHomeAlt"),
                ("CTUN", "Alt"),
                ("BARO", "Alt"),
                ("GLOBAL_POSITION_INT", "relative_alt"),
            ] {
                if let Some(t) = l.table(table).filter(|t| !t.is_empty()) {
                    let (Some(tc), Some(vc)) = (t.column(tcol), t.column(field)) else {
                        continue;
                    };
                    // GLOBAL_POSITION_INT.relative_alt is millimetres.
                    let scale = if field == "relative_alt" { 1e-3 } else { 1.0 };
                    let mut pts = Vec::new();
                    for i in 0..t.len() {
                        if let (Some(ts), Some(v)) = (tc.data.get_f64(i), vc.data.get_f64(i)) {
                            pts.push((ts as u64, v * scale));
                        }
                    }
                    if !pts.is_empty() {
                        return Some(AltSeries {
                            pts,
                            source: format!("{table}.{field}"),
                        });
                    }
                }
            }
            None
        }
        FlightLog::Px4(l) => {
            // vehicle_local_position.z is down-positive metres from the local
            // origin; height above the origin is -z where z is valid.
            let t = l.table("vehicle_local_position")?;
            let tc = t.column("timestamp")?;
            let zc = t.column("z")?;
            let valid = t.column("z_valid");
            let mut pts = Vec::new();
            for i in 0..t.len() {
                if valid.is_some_and(|c| c.data.get_i64(i) == Some(0)) {
                    continue;
                }
                if let (Some(ts), Some(z)) = (tc.data.get_f64(i), zc.data.get_f64(i)) {
                    pts.push((ts as u64, -z));
                }
            }
            (!pts.is_empty()).then_some(AltSeries {
                pts,
                source: "vehicle_local_position.z".into(),
            })
        }
    }
}

/// Greatest ground speed in m/s, with the field it came from.
fn top_speed(log: &FlightLog) -> Option<(f64, String)> {
    match log {
        FlightLog::ArduPilot(l) => {
            if let Some(t) = l.table("GPS").filter(|t| !t.is_empty()) {
                if let Some(c) = t.column("Spd") {
                    let m = (0..t.len())
                        .filter_map(|i| c.data.get_f64(i))
                        .filter(|v| v.is_finite())
                        .fold(f64::MIN, f64::max);
                    if m > f64::MIN {
                        return Some((m, "GPS.Spd".into()));
                    }
                }
            }
            if let Some(t) = l.table("VFR_HUD").filter(|t| !t.is_empty()) {
                if let Some(c) = t.column("groundspeed") {
                    let m = (0..t.len())
                        .filter_map(|i| c.data.get_f64(i))
                        .filter(|v| v.is_finite())
                        .fold(f64::MIN, f64::max);
                    if m > f64::MIN {
                        return Some((m, "VFR_HUD.groundspeed".into()));
                    }
                }
            }
            None
        }
        FlightLog::Px4(l) => {
            let t = l.table("vehicle_local_position")?;
            let (vx, vy) = (t.column("vx")?, t.column("vy")?);
            let valid = t.column("v_xy_valid");
            let mut m = f64::MIN;
            for i in 0..t.len() {
                if valid.is_some_and(|c| c.data.get_i64(i) == Some(0)) {
                    continue;
                }
                if let (Some(x), Some(y)) = (vx.data.get_f64(i), vy.data.get_f64(i)) {
                    m = m.max((x * x + y * y).sqrt());
                }
            }
            (m > f64::MIN).then_some((m, "vehicle_local_position.vx,vy".into()))
        }
    }
}

/// Great-circle distance between two lat/lon points in metres.
fn haversine_m(a: GeoPoint, b: GeoPoint) -> f64 {
    const R: f64 = 6_371_000.0;
    let (lat1, lat2) = (a.lat.to_radians(), b.lat.to_radians());
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().clamp(0.0, 1.0).asin()
}

/// Greatest distance from the takeoff point, with the source.
fn max_distance(log: &FlightLog, track: &[TrackPoint]) -> Option<(f64, String)> {
    if let Some(first) = track.first() {
        let home = GeoPoint {
            lat: first.lat,
            lon: first.lon,
        };
        let m = track
            .iter()
            .map(|p| {
                haversine_m(
                    home,
                    GeoPoint {
                        lat: p.lat,
                        lon: p.lon,
                    },
                )
            })
            .fold(0.0, f64::max);
        return Some((m, "GPS".into()));
    }
    if let FlightLog::Px4(l) = log {
        let t = l.table("vehicle_local_position")?;
        let (xc, yc) = (t.column("x")?, t.column("y")?);
        let valid = t.column("xy_valid");
        let mut m = 0.0f64;
        for i in 0..t.len() {
            if valid.is_some_and(|c| c.data.get_i64(i) == Some(0)) {
                continue;
            }
            if let (Some(x), Some(y)) = (xc.data.get_f64(i), yc.data.get_f64(i)) {
                m = m.max((x * x + y * y).sqrt());
            }
        }
        return Some((m, "vehicle_local_position.x,y".into()));
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn summary(
    log: &FlightLog,
    track: &[TrackPoint],
    alt: &Option<AltSeries>,
    start_us: u64,
    end_us: u64,
    armed_s: Option<f64>,
    flew: bool,
) -> FlightSummary {
    let max_alt = alt.as_ref().and_then(|a| {
        a.pts
            .iter()
            .map(|&(_, v)| v)
            .filter(|v| v.is_finite())
            .reduce(f64::max)
            .map(|m| (round3(m), a.source.clone()))
    });
    let dist = max_distance(log, track);
    let speed = top_speed(log);
    let takeoff = track.first().map(|p| GeoPoint {
        lat: p.lat,
        lon: p.lon,
    });
    let landing = track.last().map(|p| GeoPoint {
        lat: p.lat,
        lon: p.lon,
    });
    let utc_start = track
        .first()
        .and_then(|p| p.utc_us)
        .map(crate::track::iso8601_utc);

    FlightSummary {
        format: log.format_name().into(),
        log_duration_s: round3((end_us.saturating_sub(start_us)) as f64 / 1e6),
        armed_s,
        flew,
        max_alt_agl_m: max_alt.as_ref().map(|(m, _)| *m),
        alt_source: max_alt.as_ref().map(|(_, s)| s.clone()),
        max_distance_m: dist.as_ref().map(|(m, _)| round3(*m)),
        distance_source: dist.map(|(_, s)| s),
        top_speed_ms: speed.as_ref().map(|(m, _)| round3(*m)),
        speed_source: speed.map(|(_, s)| s),
        takeoff,
        landing,
        utc_start,
        gps_fixes: track.len(),
    }
}

/// Lower-case match of a return/land flight-mode name.
fn is_landing_mode(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "rtl",
        "land",
        "smart_rtl",
        "smartrtl",
        "auto_rtl",
        "qrtl",
        "qland",
        "return",
        "descend",
    ]
    .iter()
    .any(|k| n == *k || n.contains(k))
}

/// True for an event that reports a failsafe being resolved or cleared,
/// as either a DataFlash `ERR` ("... Failsafe Resolved") or a text message
/// ("... Failsafe Cleared", PX4 `FailsafeCleared`).
fn is_failsafe_resolve(e: &FlightEvent) -> bool {
    match e.kind {
        EventKind::FailsafeCleared => true,
        EventKind::Error | EventKind::Message => {
            let has_fs = e.text.to_ascii_lowercase().contains("failsafe");
            has_fs && (e.text.contains("Resolved") || e.text.contains("Cleared"))
        }
        _ => false,
    }
}

/// True for an event that reports a failsafe being triggered. DataFlash logs
/// it as "... Failsafe: Failsafe Triggered", tlogs as a "... Failsafe" status
/// text; PX4 as a `Failsafe` event.
fn is_failsafe_trigger(e: &FlightEvent) -> bool {
    match e.kind {
        EventKind::Failsafe => true,
        EventKind::Error | EventKind::Message => {
            e.text.to_ascii_lowercase().contains("failsafe") && !is_failsafe_resolve(e)
        }
        _ => false,
    }
}

/// The failsafe family from an event text: the text up to and including the
/// word "Failsafe" (`"Radio Failsafe: Failsafe Triggered"` and
/// `"GCS Failsafe - Continuing Landing"` both give `"GCS/Radio Failsafe"`),
/// or `"failsafe"` for a PX4 failsafe event with no family.
fn failsafe_family(e: &FlightEvent) -> String {
    if e.kind == EventKind::Failsafe {
        return "failsafe".into();
    }
    let lower = e.text.to_ascii_lowercase();
    match lower.find("failsafe") {
        Some(i) => {
            let end = i + "failsafe".len();
            let head = e.text[..end].trim();
            if head.is_empty() {
                "failsafe".into()
            } else {
                head.to_string()
            }
        }
        None => "failsafe".into(),
    }
}

/// Greatest sustained descent rate (m/s) in the final window, and the height
/// lost over it, from the height series while the vehicle is armed.
fn terminal_descent(alt: &AltSeries, armed: &[(u64, u64)], window_us: u64) -> Option<(f64, f64)> {
    let (_, flight_end) = armed.last().copied()?;
    let in_armed = |t: u64| armed.iter().any(|&(s, e)| t >= s && t <= e);
    let win: Vec<(u64, f64)> = alt
        .pts
        .iter()
        .copied()
        .filter(|&(t, _)| t + window_us >= flight_end && t <= flight_end && in_armed(t))
        .collect();
    if win.len() < 2 {
        return None;
    }
    let first = win.first()?.1;
    let last = win.last()?.1;
    let dt = (win.last()?.0 - win.first()?.0) as f64 / 1e6;
    if dt <= 0.0 {
        return None;
    }
    let drop = first - last;
    if drop <= 0.0 {
        return None;
    }
    Some((drop / dt, drop))
}

#[allow(clippy::too_many_arguments)]
fn ending(
    log: &FlightLog,
    events: &[FlightEvent],
    alt: &Option<AltSeries>,
    armed: &[(u64, u64)],
    start_us: u64,
    end_us: u64,
    ended_armed: bool,
    opts: &ReportOptions,
) -> Ending {
    let flight_end = armed.last().map(|&(_, e)| e).unwrap_or(end_us);
    let flight_start = armed.first().map(|&(s, _)| s).unwrap_or(start_us);
    let in_flight = |t: u64| t >= flight_start && t <= flight_end;
    let truncated = log.integrity().truncated_tail_bytes > 0;

    let mut evidence: Vec<Evidence> = Vec::new();
    let ev = |evidence: &mut Vec<Evidence>, t: u64, detail: String| {
        evidence.push(Evidence {
            t_s: to_s(t, start_us),
            detail,
        });
    };

    // Final flight mode.
    let final_mode = events
        .iter()
        .rfind(|e| e.kind == EventKind::Mode && in_flight(e.time_us))
        .map(|e| e.text.clone());

    // Active failsafes at the end of the flight: triggers with no later
    // resolve, counted per family.
    let mut active: std::collections::BTreeMap<String, (i64, u64)> = Default::default();
    for e in events.iter().filter(|e| in_flight(e.time_us)) {
        if is_failsafe_trigger(e) {
            let f = failsafe_family(e);
            let entry = active.entry(f).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = e.time_us;
        } else if is_failsafe_resolve(e) {
            let f = failsafe_family(e);
            if let Some(entry) = active.get_mut(&f) {
                entry.0 = (entry.0 - 1).max(0);
            }
        }
    }
    let active_fs = active
        .iter()
        .filter(|(_, (n, _))| *n > 0)
        .max_by_key(|(_, (_, t))| *t)
        .map(|(f, (_, t))| (f.clone(), *t));

    // Crash signatures.
    let crash_text = events.iter().find(|e| {
        in_flight(e.time_us)
            && matches!(e.kind, EventKind::Message | EventKind::Error)
            && e.text.to_ascii_lowercase().contains("crash")
    });
    let fd_trip = events
        .iter()
        .find(|e| in_flight(e.time_us) && e.kind == EventKind::FailureDetector);
    let window_us = (opts.end_window_s * 1e6) as u64;
    let descent = alt
        .as_ref()
        .filter(|_| ended_armed && !armed.is_empty())
        .and_then(|a| terminal_descent(a, armed, window_us));

    // Height at the end of the flight.
    let end_alt = alt.as_ref().and_then(|a| {
        a.pts
            .iter()
            .rfind(|&&(t, _)| t <= flight_end)
            .map(|&(_, v)| v)
    });

    // Classify, most serious first.
    let (cause, confidence, mut summary, failsafe) = if let Some(c) = crash_text {
        ev(
            &mut evidence,
            c.time_us,
            format!("crash message: {}", c.text),
        );
        (
            EndCause::CrashSignature,
            Confidence::Reported,
            format!(
                "The flight ended in a crash: the firmware logged \"{}\".",
                c.text
            ),
            None,
        )
    } else if let Some(fd) = fd_trip {
        ev(
            &mut evidence,
            fd.time_us,
            format!("failure detector: {}", fd.text),
        );
        (
            EndCause::CrashSignature,
            Confidence::Reported,
            format!(
                "The flight ended with a failure-detector trip: {}.",
                fd.text
            ),
            None,
        )
    } else if let Some((rate, drop)) = descent.filter(|&(r, _)| r >= opts.crash_descent_ms) {
        ev(
            &mut evidence,
            flight_end,
            format!("height fell {drop:.1} m at {rate:.1} m/s while still armed"),
        );
        (
            EndCause::CrashSignature,
            Confidence::Inferred,
            format!(
                "Possible crash: height fell {drop:.1} m at {rate:.1} m/s in the last {:.0} s while the vehicle was still armed.",
                opts.end_window_s
            ),
            None,
        )
    } else if ended_armed && end_alt.is_some_and(|a| a > opts.airborne_end_m) {
        let a = clean0(end_alt.unwrap_or(0.0));
        ev(
            &mut evidence,
            flight_end,
            format!("log ends while armed at {a:.0} m above takeoff"),
        );
        if truncated {
            ev(
                &mut evidence,
                end_us,
                format!(
                    "{} bytes at the end did not form a message",
                    log.integrity().truncated_tail_bytes
                ),
            );
        }
        (
            EndCause::EndedAirborne,
            Confidence::Inferred,
            format!(
                "The log ends while the vehicle is still armed at {a:.0} m above takeoff: the recording{} stopped in flight.",
                if truncated { " (the file is truncated)" } else { "" }
            ),
            None,
        )
    } else if let Some((family, t)) = active_fs {
        ev(
            &mut evidence,
            t,
            format!("{family} triggered and was not resolved before the flight ended"),
        );
        // The failsafe forced the end only when a return/land mode was entered
        // at or after it triggered; a landing mode entered earlier (a mission
        // landing, say) was not caused by this failsafe.
        let forced_mode = final_mode
            .as_deref()
            .filter(|m| is_landing_mode(m))
            .and_then(|m| {
                events
                    .iter()
                    .rev()
                    .find(|e| {
                        e.kind == EventKind::Mode
                            && in_flight(e.time_us)
                            && e.text == m
                            && e.time_us >= t
                    })
                    .map(|e| (m.to_string(), e.time_us))
            });
        let summary = match &forced_mode {
            Some((m, me)) => {
                ev(
                    &mut evidence,
                    *me,
                    format!("entered {m} after the failsafe"),
                );
                format!("The flight ended because of a {family}: the vehicle went to {m} and came down.")
            }
            None => match &final_mode {
                Some(m) => format!(
                    "The flight ended with a {family} active; the vehicle was in {m} and disarmed."
                ),
                None => format!("The flight ended with a {family} active."),
            },
        };
        (
            EndCause::FailsafeLanding,
            Confidence::Reported,
            summary,
            Some(family),
        )
    } else if !armed.is_empty() && !ended_armed {
        if let Some(d) = events
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::Disarmed && in_flight(e.time_us))
        {
            ev(&mut evidence, d.time_us, "disarmed".into());
        }
        let alt_note = match end_alt {
            Some(a) => format!(" at {:.0} m above takeoff", clean0(a)),
            None => String::new(),
        };
        (
            EndCause::NormalLanding,
            Confidence::Reported,
            format!("Normal end of flight: the vehicle landed and disarmed{alt_note}, with no failsafe active."),
            None,
        )
    } else {
        (
            EndCause::Undetermined,
            Confidence::Inferred,
            if armed.is_empty() {
                "No arming is logged, so the end of flight cannot be determined.".into()
            } else {
                "The end of flight could not be attributed to a single cause.".into()
            },
            None,
        )
    };

    // Note a failsafe that occurred and recovered during the flight, when it
    // is not itself the end cause.
    if cause != EndCause::FailsafeLanding {
        if let Some(e) = events
            .iter()
            .find(|e| in_flight(e.time_us) && is_failsafe_trigger(e))
        {
            let f = failsafe_family(e);
            summary.push_str(&format!(" A {f} occurred earlier in the flight."));
            ev(
                &mut evidence,
                e.time_us,
                format!("{f} triggered (recovered)"),
            );
        }
    }

    evidence.sort_by(|a, b| {
        a.t_s
            .partial_cmp(&b.t_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ending {
        cause,
        confidence,
        summary,
        failsafe,
        final_mode,
        ended_armed,
        truncated,
        evidence,
    }
}

fn altitude(alt: &Option<AltSeries>, start_us: u64, opts: &ReportOptions) -> AltitudeCompliance {
    let ceiling = opts.ceiling_agl_m;
    let ceiling_ft = round3(ceiling * 3.280_84);
    let Some(a) = alt else {
        return AltitudeCompliance {
            ceiling_agl_m: round6(ceiling),
            ceiling_agl_ft: ceiling_ft,
            exceeded: false,
            max_alt_agl_m: None,
            alt_source: None,
            ranges: Vec::new(),
            summary: "No height data in this log: altitude compliance cannot be checked.".into(),
            citation: PART_107_CITATION.into(),
        };
    };
    let max_alt = a
        .pts
        .iter()
        .map(|&(_, v)| v)
        .filter(|v| v.is_finite())
        .reduce(f64::max);

    // Contiguous runs above the ceiling.
    let mut ranges: Vec<CeilingRange> = Vec::new();
    let mut cur: Option<(u64, u64, f64)> = None;
    for &(t, v) in &a.pts {
        if v.is_finite() && v > ceiling {
            cur = Some(match cur {
                Some((s, _, peak)) => (s, t, peak.max(v)),
                None => (t, t, v),
            });
        } else if let Some((s, e, peak)) = cur.take() {
            ranges.push(CeilingRange {
                start_s: to_s(s, start_us),
                end_s: to_s(e, start_us),
                peak_m: round3(peak),
            });
        }
    }
    if let Some((s, e, peak)) = cur {
        ranges.push(CeilingRange {
            start_s: to_s(s, start_us),
            end_s: to_s(e, start_us),
            peak_m: round3(peak),
        });
    }

    let exceeded = !ranges.is_empty();
    let summary = if exceeded {
        let peak = ranges.iter().map(|r| r.peak_m).fold(0.0, f64::max);
        format!(
            "Exceeded the {:.0} ft ({:.0} m) ceiling: peak {:.0} m ({:.0} ft) above takeoff, in {} range(s).",
            ceiling_ft,
            ceiling,
            peak,
            peak * 3.280_84,
            ranges.len()
        )
    } else {
        match max_alt {
            Some(m) => format!(
                "Stayed under the {:.0} ft ({:.0} m) ceiling: peak {:.0} m ({:.0} ft) above takeoff.",
                ceiling_ft,
                ceiling,
                clean0(m),
                clean0(m) * 3.280_84
            ),
            None => format!("Stayed under the {ceiling_ft:.0} ft ceiling."),
        }
    };

    AltitudeCompliance {
        ceiling_agl_m: round6(ceiling),
        ceiling_agl_ft: ceiling_ft,
        exceeded,
        max_alt_agl_m: max_alt.map(round3),
        alt_source: Some(a.source.clone()),
        ranges,
        summary,
        citation: PART_107_CITATION.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_known_distance() {
        // One degree of latitude is about 111.19 km on a sphere of radius
        // 6_371_000 m (pi * R / 180).
        let d = haversine_m(
            GeoPoint { lat: 0.0, lon: 0.0 },
            GeoPoint { lat: 1.0, lon: 0.0 },
        );
        assert!((d - 111_194.9).abs() < 1.0, "got {d}");
        // Same point is zero.
        let z = haversine_m(
            GeoPoint {
                lat: 35.0,
                lon: -120.0,
            },
            GeoPoint {
                lat: 35.0,
                lon: -120.0,
            },
        );
        assert!(z < 1e-6, "got {z}");
    }

    #[test]
    fn landing_mode_names() {
        assert!(is_landing_mode("RTL"));
        assert!(is_landing_mode("LAND"));
        assert!(is_landing_mode("SMART_RTL"));
        assert!(is_landing_mode("QRTL"));
        assert!(is_landing_mode("Return"));
        assert!(!is_landing_mode("AUTO"));
        assert!(!is_landing_mode("LOITER"));
        assert!(!is_landing_mode("STABILIZE"));
    }

    #[test]
    fn terminal_descent_detects_fall() {
        // Height holds at 50 m, then falls to 2 m over the last 2 s: a 48 m
        // drop at 24 m/s. Timestamps in microseconds.
        let pts = vec![
            (0, 50.0),
            (1_000_000, 50.0),
            (2_000_000, 50.0),
            (3_000_000, 50.0),
            (4_000_000, 26.0),
            (5_000_000, 2.0),
        ];
        let alt = AltSeries {
            pts,
            source: "test".into(),
        };
        let armed = [(0u64, 5_000_000u64)];
        let (rate, drop) = terminal_descent(&alt, &armed, 3_000_000).expect("descent");
        assert!((drop - 48.0).abs() < 1e-6, "drop {drop}");
        assert!((rate - 16.0).abs() < 1e-6, "rate {rate}");
    }

    #[test]
    fn terminal_descent_ignores_climb_and_level() {
        let alt = AltSeries {
            pts: vec![(0, 10.0), (1_000_000, 20.0), (2_000_000, 30.0)],
            source: "test".into(),
        };
        let armed = [(0u64, 2_000_000u64)];
        assert!(terminal_descent(&alt, &armed, 3_000_000).is_none());
    }

    #[test]
    fn failsafe_family_parsing() {
        let e = FlightEvent {
            time_us: 0,
            kind: EventKind::Error,
            text: "Radio Failsafe: Failsafe Triggered".into(),
            code: None,
        };
        assert!(is_failsafe_trigger(&e));
        assert!(!is_failsafe_resolve(&e));
        assert_eq!(failsafe_family(&e), "Radio Failsafe");
    }
}

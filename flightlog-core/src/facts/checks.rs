//! The checks. Each returns one [`Check`]; see docs/flight-facts.md for the
//! exact rule of each.

use std::cell::RefCell;

use serde_json::{json, Map, Value};

use super::series::{armed_ranges, distinct, instance_rows, total_s, vehicle_sys_id, Series};
use super::wiki::{self, Cite};
use super::{Check, Event, LogInfo, Options, Range, Source, Status, Threshold};
use crate::{Log, LogKind, Table};

pub(crate) struct Ctx<'a> {
    pub log: &'a Log,
    pub opts: &'a Options,
    pub start_us: u64,
    pub end_us: u64,
    pub armed: Vec<(u64, u64)>,
    pub armed_source: &'static str,
    pub params_used: RefCell<Map<String, Value>>,
}

impl<'a> Ctx<'a> {
    pub fn new(log: &'a Log, opts: &'a Options) -> Self {
        let (start_us, end_us) = log.time_range_us().unwrap_or((0, 0));
        let (armed, armed_source) = armed_ranges(log, end_us);
        Ctx {
            log,
            opts,
            start_us,
            end_us,
            armed,
            armed_source,
            params_used: RefCell::new(Map::new()),
        }
    }

    pub fn range(&self, s: u64, e: u64, detail: impl Into<String>) -> Range {
        Range {
            start_us: s,
            end_us: e,
            start_s: round3(s.saturating_sub(self.start_us) as f64 / 1e6),
            end_s: round3(e.saturating_sub(self.start_us) as f64 / 1e6),
            detail: detail.into(),
        }
    }

    /// Armed ranges, or the whole log when arming is not logged.
    fn flight(&self) -> Vec<(u64, u64)> {
        if self.armed_source == "none" {
            vec![(self.start_us, self.end_us)]
        } else {
            self.armed.clone()
        }
    }

    fn param(&self, name: &str, default: Option<f64>) -> Option<(f64, &'static str)> {
        let v = match self.log.param(name) {
            Some(v) => Some((v, "param")),
            None => default.map(|d| (d, "param_default")),
        };
        if let Some((val, origin)) = v {
            self.params_used
                .borrow_mut()
                .insert(name.into(), json!({ "value": num(val), "origin": origin }));
        }
        v
    }

    fn table(&self, name: &str) -> Option<&'a Table> {
        self.log.table(name).filter(|t| !t.is_empty())
    }

    pub fn vehicle(&self) -> Option<String> {
        match self.log.kind {
            LogKind::DataFlash => {
                if let Some(bu) = self
                    .table("VER")
                    .and_then(|t| t.column("BU"))
                    .and_then(|c| c.data.get_i64(0))
                {
                    let v = match bu {
                        1 => "rover",
                        2 => "copter",
                        3 => "plane",
                        4 => "tracker",
                        7 => "sub",
                        12 => "blimp",
                        13 => "heli",
                        _ => "",
                    };
                    if !v.is_empty() {
                        return Some(v.into());
                    }
                }
                let fw = self.firmware()?;
                [
                    ("ArduCopter", "copter"),
                    ("ArduPlane", "plane"),
                    ("ArduRover", "rover"),
                    ("ArduSub", "sub"),
                    ("Blimp", "blimp"),
                    ("AntennaTracker", "tracker"),
                ]
                .iter()
                .find(|(k, _)| fw.contains(k))
                .map(|(_, v)| (*v).to_string())
            }
            LogKind::Tlog => {
                let hb = self.table("HEARTBEAT")?;
                let sys = vehicle_sys_id(self.log)?;
                let (sid, ty) = (hb.column("sys_id")?, hb.column("type")?);
                let i = (0..hb.len()).find(|&i| sid.data.get_i64(i) == Some(sys))?;
                Some(
                    match ty.data.get_i64(i)? {
                        1 => "plane",
                        2 | 3 | 13 | 14 | 15 => "copter",
                        4 => "heli",
                        10 | 11 => "rover",
                        12 => "sub",
                        5 => "tracker",
                        7 => "blimp",
                        19..=25 => "plane",
                        _ => return None,
                    }
                    .into(),
                )
            }
        }
    }

    pub fn firmware(&self) -> Option<String> {
        match self.log.kind {
            LogKind::DataFlash => {
                if let Some(s) = self
                    .table("VER")
                    .and_then(|t| t.column("FWS"))
                    .and_then(|c| c.data.get_str(0).map(String::from))
                {
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
                let msg = self.table("MSG")?.column("Message")?;
                (0..msg.data.len())
                    .filter_map(|i| msg.data.get_str(i))
                    .find(|m| {
                        m.starts_with("Ardu")
                            || m.starts_with("Blimp")
                            || m.starts_with("AntennaTracker")
                    })
                    .map(String::from)
            }
            LogKind::Tlog => {
                let st = self.table("STATUSTEXT")?.column("text")?;
                (0..st.data.len())
                    .filter_map(|i| st.data.get_str(i))
                    .find(|m| m.starts_with("Ardu") && m.contains(" V"))
                    .map(String::from)
            }
        }
    }

    pub fn log_info(&self) -> LogInfo {
        let range = self.log.time_range_us();
        let utc = match (self.log.kind, self.log.utc_offset_us, range) {
            (LogKind::DataFlash, Some(off), Some((s, _))) => Some(super::iso8601(off + s as i64)),
            (LogKind::Tlog, _, Some((s, _))) => Some(super::iso8601(s as i64)),
            _ => None,
        };
        LogInfo {
            kind: super::kind_name(self.log.kind).into(),
            vehicle: self.vehicle(),
            firmware: self.firmware(),
            time_base: match self.log.kind {
                LogKind::DataFlash => "boot_us".into(),
                LogKind::Tlog => "unix_us".into(),
            },
            start_us: range.map(|r| r.0),
            end_us: range.map(|r| r.1),
            duration_s: range.map_or(0.0, |r| round3((r.1 - r.0) as f64 / 1e6)),
            utc_start: utc,
            message_types: self.log.counts.len(),
            messages: self.log.integrity.messages,
        }
    }

    /// ERR rows as (time, subsystem, code).
    fn errs(&self) -> Vec<(u64, i64, i64)> {
        let Some(t) = self.table("ERR") else {
            return Vec::new();
        };
        let (Some(tc), Some(s), Some(c)) =
            (t.column("TimeUS"), t.column("Subsys"), t.column("ECode"))
        else {
            return Vec::new();
        };
        (0..t.len())
            .filter_map(|i| {
                Some((
                    tc.data.get_f64(i)? as u64,
                    s.data.get_i64(i)?,
                    c.data.get_i64(i)?,
                ))
            })
            .collect()
    }

    /// tlog STATUSTEXT rows from the vehicle.
    fn texts(&self) -> Vec<(u64, String)> {
        let Some(t) = self.table("STATUSTEXT") else {
            return Vec::new();
        };
        let sys = vehicle_sys_id(self.log);
        let (Some(tc), Some(x), Some(sid)) = (
            t.column("timestamp_us"),
            t.column("text"),
            t.column("sys_id"),
        ) else {
            return Vec::new();
        };
        (0..t.len())
            .filter(|&i| sys.is_none() || sid.data.get_i64(i) == sys)
            .filter_map(|i| Some((tc.data.get_f64(i)? as u64, x.data.get_str(i)?.to_string())))
            .collect()
    }

    /// Ranges between an ERR "on" code and the next "off" code of a subsystem.
    fn err_ranges(&self, subsys: i64, on: &[i64], off: i64) -> Vec<(u64, u64, i64)> {
        let mut out = Vec::new();
        let mut cur: Option<(u64, i64)> = None;
        for (t, s, c) in self.errs() {
            if s != subsys {
                continue;
            }
            if on.contains(&c) {
                if cur.is_none() {
                    cur = Some((t, c));
                }
            } else if c == off {
                if let Some((st, code)) = cur.take() {
                    out.push((st, t, code));
                }
            }
        }
        if let Some((st, code)) = cur {
            out.push((st, self.end_us, code));
        }
        out
    }

    /// Ranges between texts that start with `on` and texts that start with `off`.
    fn text_ranges(
        &self,
        on: &dyn Fn(&str) -> bool,
        off: &dyn Fn(&str) -> bool,
    ) -> Vec<(u64, u64, String)> {
        let mut out = Vec::new();
        let mut cur: Option<(u64, String)> = None;
        for (t, x) in self.texts() {
            if off(&x) {
                if let Some((s, tx)) = cur.take() {
                    out.push((s, t, tx));
                }
            } else if on(&x) && cur.is_none() {
                cur = Some((t, x));
            }
        }
        if let Some((s, tx)) = cur {
            out.push((s, self.end_us, tx));
        }
        out
    }

    pub fn events(&self) -> Vec<Event> {
        let mut ev = Vec::new();
        let mk = |t: u64, kind: &str, text: String, code: Option<i64>, sub: Option<i64>| Event {
            time_us: t,
            t_s: round3(t.saturating_sub(self.start_us) as f64 / 1e6),
            kind: kind.into(),
            text,
            code,
            subcode: sub,
        };
        match self.log.kind {
            LogKind::DataFlash => {
                for (t, s, c) in self.errs() {
                    let text = match wiki::err_code(s, c) {
                        Some(m) => format!("{}: {}", wiki::err_subsystem(s), m),
                        None => format!("{}: code {}", wiki::err_subsystem(s), c),
                    };
                    ev.push(mk(t, "error", text, Some(s), Some(c)));
                }
                if let Some(m) = self.table("MODE") {
                    let tc = m.column("TimeUS");
                    let mc = m.column("ModeNum").or_else(|| m.column("Mode"));
                    if let (Some(tc), Some(mc)) = (tc, mc) {
                        let veh = self.vehicle();
                        let mut last: Option<i64> = None;
                        for i in 0..m.len() {
                            if let (Some(t), Some(n)) = (tc.data.get_f64(i), mc.data.get_i64(i)) {
                                // MODE is logged again on re-entry and at log start: keep changes only
                                if last == Some(n) {
                                    continue;
                                }
                                last = Some(n);
                                let name = mode_name(veh.as_deref(), n)
                                    .map(String::from)
                                    .unwrap_or_else(|| format!("mode {n}"));
                                ev.push(mk(t as u64, "mode", name, Some(n), None));
                            }
                        }
                    }
                }
            }
            LogKind::Tlog => {
                for (t, x) in self.texts() {
                    if is_failsafe_text(&x) {
                        ev.push(mk(t, "text", x, None, None));
                    }
                }
            }
        }
        for &(s, e) in &self.armed {
            ev.push(mk(s, "armed", "armed".into(), None, None));
            if e < self.end_us || self.log.kind == LogKind::Tlog {
                ev.push(mk(e, "disarmed", "disarmed".into(), None, None));
            }
        }
        ev.sort_by_key(|e| e.time_us);
        ev
    }
}

fn is_failsafe_text(x: &str) -> bool {
    x.contains("Failsafe")
        || x.starts_with("EKF variance")
        || x.starts_with("Vibration compensation")
        || x.starts_with("GPS Glitch")
        || x == "Glitch cleared"
        || x.starts_with("Crash")
}

pub(crate) fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(round6(x))
    } else {
        Value::Null
    }
}

fn round6(x: f64) -> f64 {
    if x.abs() >= 1e9 {
        x
    } else {
        (x * 1e6).round() / 1e6
    }
}

fn opt(x: Option<f64>) -> Value {
    x.map_or(Value::Null, num)
}

fn th(name: &str, value: f64, unit: &str, origin: &str) -> Threshold {
    Threshold {
        name: name.into(),
        value,
        unit: unit.into(),
        origin: origin.into(),
    }
}

fn src(c: &[Cite]) -> Vec<Source> {
    c.iter().map(|&c| c.into()).collect()
}

fn check(id: &str, status: Status, summary: String) -> Check {
    Check {
        id: id.into(),
        status,
        summary,
        thresholds: Vec::new(),
        measured: Map::new(),
        ranges: Vec::new(),
        evidence: Vec::new(),
        sources: Vec::new(),
    }
}

fn na(id: &str, what: &str, sources: &[Cite]) -> Check {
    let mut c = check(id, Status::NotApplicable, format!("No {what} in this log."));
    c.sources = src(sources);
    c
}

pub(crate) fn all(ctx: &Ctx) -> Vec<Check> {
    vec![
        vibration_levels(ctx),
        vibration_clipping(ctx),
        vibration_failsafe(ctx),
        ekf_variance(ctx),
        gps_hdop(ctx),
        gps_satellites(ctx),
        gps_glitch(ctx),
        battery_voltage(ctx),
        power_board_vcc(ctx),
        power_log_end(ctx),
        rc_failsafe(ctx),
        gcs_failsafe(ctx),
    ]
}

/// A time range and what happened in it.
type TextRange = (u64, u64, String);

/// Instance label and x, y, z series.
type Axes = (String, [Series; 3]);

/// (instance label, x, y, z series) for vibration.
fn vibe_series(ctx: &Ctx) -> Option<(Vec<Axes>, Vec<String>)> {
    let log = ctx.log;
    match log.kind {
        LogKind::DataFlash => {
            let t = ctx.table("VIBE")?;
            let insts = if t.column("IMU").is_some() {
                distinct(t, "IMU")
            } else {
                vec![0]
            };
            let mut out = Vec::new();
            for inst in insts {
                let rows = instance_rows(t, "IMU", inst);
                let s = |f: &str| Series::from_table(log, t, f, Some(&rows));
                out.push((
                    format!("IMU{inst}"),
                    [s("VibeX")?, s("VibeY")?, s("VibeZ")?],
                ));
            }
            Some((
                out,
                vec![
                    "VIBE.VibeX".into(),
                    "VIBE.VibeY".into(),
                    "VIBE.VibeZ".into(),
                ],
            ))
        }
        LogKind::Tlog => {
            let t = ctx.table("VIBRATION")?;
            let sys = vehicle_sys_id(log);
            let rows: Vec<usize> = match (sys, t.column("sys_id")) {
                (Some(s), Some(c)) => (0..t.len())
                    .filter(|&i| c.data.get_i64(i) == Some(s))
                    .collect(),
                _ => (0..t.len()).collect(),
            };
            let s = |f: &str| Series::from_table(log, t, f, Some(&rows));
            Some((
                vec![(
                    "primary".into(),
                    [s("vibration_x")?, s("vibration_y")?, s("vibration_z")?],
                )],
                vec![
                    "VIBRATION.vibration_x".into(),
                    "VIBRATION.vibration_y".into(),
                    "VIBRATION.vibration_z".into(),
                ],
            ))
        }
    }
}

fn vibration_levels(ctx: &Ctx) -> Check {
    const ID: &str = "vibration.levels";
    let Some((imus, evidence)) = vibe_series(ctx) else {
        return na(
            ID,
            "vibration data (VIBE / VIBRATION)",
            &[wiki::VIBE_LEVELS],
        );
    };
    let flight = ctx.flight();
    let p = ctx.opts.vibe_percentile;
    let mut status = Status::Pass;
    let mut measured = Map::new();
    let mut ranges = Vec::new();
    let mut worst_p = 0.0f64;
    let mut worst_max = 0.0f64;
    let mut samples = 0usize;
    for (label, axes) in &imus {
        let mut m = Map::new();
        for (axis, s) in ["x", "y", "z"].iter().zip(axes.iter()) {
            let s = s.within(&flight);
            samples += s.len();
            let pv = s.percentile(p);
            let mx = s.max();
            worst_p = worst_p.max(pv.unwrap_or(0.0));
            worst_max = worst_max.max(mx.unwrap_or(0.0));
            m.insert(format!("{axis}_max"), opt(mx));
            m.insert(format!("{axis}_p{}", (p * 100.0).round()), opt(pv));
            for (lim, tag) in [(60.0, "above 60 m/s/s"), (30.0, "above 30 m/s/s")] {
                for (a, b) in s.runs(|v| v > lim, ctx.opts.merge_gap_us) {
                    ranges.push(ctx.range(a, b, format!("{label} {axis} {tag}")));
                }
            }
        }
        measured.insert(label.clone(), Value::Object(m));
    }
    if samples == 0 {
        return na(ID, "vibration samples while armed", &[wiki::VIBE_LEVELS]);
    }
    let pct = (p * 100.0).round();
    let summary = if worst_p > 60.0 {
        status = Status::Fail;
        format!("Vibration p{pct} is {worst_p:.1} m/s/s, above the wiki's 60 m/s/s level that nearly always causes position or altitude hold problems.")
    } else if worst_p > 30.0 {
        status = Status::Warn;
        format!("Vibration p{pct} is {worst_p:.1} m/s/s, above the wiki's 30 m/s/s level that may cause problems.")
    } else if worst_max > 30.0 {
        status = Status::Info;
        format!("Vibration is mostly below 30 m/s/s (p{pct} {worst_p:.1}) with peaks to {worst_max:.1} m/s/s.")
    } else {
        format!("Vibration stays below 30 m/s/s (max {worst_max:.1} m/s/s).")
    };
    let mut c = check(ID, status, summary);
    c.thresholds = vec![
        th("warn", 30.0, "m/s/s", "wiki"),
        th("fail", 60.0, "m/s/s", "wiki"),
        th("percentile", p, "", "flightlog option"),
    ];
    c.measured = measured;
    c.ranges = ranges;
    c.evidence = evidence;
    c.sources = src(&[wiki::VIBE_LEVELS, wiki::VIBE_MOSTLY]);
    c
}

fn vibration_clipping(ctx: &Ctx) -> Check {
    const ID: &str = "vibration.clipping";
    let log = ctx.log;
    // (label, cumulative clip counter series)
    let mut counters: Vec<(String, Series)> = Vec::new();
    let mut evidence = Vec::new();
    match log.kind {
        LogKind::DataFlash => {
            let Some(t) = ctx.table("VIBE") else {
                return na(ID, "VIBE messages", &[wiki::VIBE_CLIP]);
            };
            if t.column("Clip").is_some() {
                for inst in if t.column("IMU").is_some() {
                    distinct(t, "IMU")
                } else {
                    vec![0]
                } {
                    let rows = instance_rows(t, "IMU", inst);
                    if let Some(s) = Series::from_table(log, t, "Clip", Some(&rows)) {
                        counters.push((format!("IMU{inst}"), s));
                    }
                }
                evidence.push("VIBE.Clip".into());
            } else {
                for k in 0..3 {
                    let f = format!("Clip{k}");
                    if let Some(s) = Series::from_table(log, t, &f, None) {
                        counters.push((format!("IMU{k}"), s));
                        evidence.push(format!("VIBE.{f}"));
                    }
                }
            }
        }
        LogKind::Tlog => {
            let Some(t) = ctx.table("VIBRATION") else {
                return na(ID, "VIBRATION messages", &[wiki::VIBE_CLIP]);
            };
            for k in 0..3 {
                let f = format!("clipping_{k}");
                if let Some(s) = Series::from_table(log, t, &f, None) {
                    counters.push((format!("IMU{k}"), s));
                    evidence.push(format!("VIBRATION.{f}"));
                }
            }
        }
    }
    if counters.is_empty() {
        return na(ID, "clipping counters", &[wiki::VIBE_CLIP]);
    }
    let mut measured = Map::new();
    let mut ranges = Vec::new();
    let mut worst = 0.0f64;
    for (label, s) in &counters {
        // Counters are cumulative since boot.
        let first = s.v.first().copied().unwrap_or(0.0);
        let total = s.max().unwrap_or(0.0);
        worst = worst.max(total);
        measured.insert(format!("{label}_total"), num(total));
        // ranges where the counter rose
        let mut prev = first;
        let mut rising = Series::default();
        for (i, &v) in s.v.iter().enumerate() {
            rising.t.push(s.t[i]);
            rising.v.push(if v > prev { 1.0 } else { 0.0 });
            prev = v;
        }
        for (a, b) in rising.runs(|v| v > 0.0, ctx.opts.merge_gap_us) {
            ranges.push(ctx.range(a, b, format!("{label} clip count rising")));
        }
    }
    let (status, summary) = if worst == 0.0 {
        (Status::Pass, "No accelerometer clipping.".to_string())
    } else if worst < 100.0 {
        (Status::Info, format!("Accelerometer clipping count reached {worst:.0}; the wiki calls low numbers (<100) likely ok."))
    } else {
        (Status::Warn, format!("Accelerometer clipping count reached {worst:.0} (100 or more); the wiki says a regularly increasing count indicates a serious vibration problem."))
    };
    let mut c = check(ID, status, summary);
    c.thresholds = vec![
        th("info", 1.0, "count", "wiki"),
        th("warn", 100.0, "count", "wiki"),
    ];
    c.measured = measured;
    c.ranges = ranges;
    c.evidence = evidence;
    c.sources = src(&[wiki::VIBE_CLIP]);
    c
}

fn vibration_failsafe(ctx: &Ctx) -> Check {
    const ID: &str = "vibration.failsafe";
    let ranges: Vec<(u64, u64, String)> = match ctx.log.kind {
        LogKind::DataFlash => {
            if ctx.log.counts.is_empty() {
                return na(ID, "messages", &[wiki::VIBE_FAILSAFE]);
            }
            ctx.err_ranges(29, &[1], 0)
                .into_iter()
                .map(|(a, b, _)| {
                    (
                        a,
                        b,
                        "ERR Vibration Failsafe: compensation active".to_string(),
                    )
                })
                .collect()
        }
        LogKind::Tlog => ctx.text_ranges(
            &|x: &str| x.starts_with("Vibration compensation ON"),
            &|x: &str| x.starts_with("Vibration compensation OFF"),
        ),
    };
    let mut c = if ranges.is_empty() {
        check(
            ID,
            Status::Pass,
            "The vibration failsafe did not trigger.".into(),
        )
    } else {
        check(
            ID,
            Status::Warn,
            format!(
                "The vibration failsafe (vibration compensation) was active {} time(s), {:.1} s in total.",
                ranges.len(),
                total_s(&ranges.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>())
            ),
        )
    };
    c.ranges = ranges
        .iter()
        .map(|(a, b, d)| ctx.range(*a, *b, d.clone()))
        .collect();
    c.measured.insert("activations".into(), json!(ranges.len()));
    c.evidence = vec![match ctx.log.kind {
        LogKind::DataFlash => "ERR.Subsys=29".into(),
        LogKind::Tlog => "STATUSTEXT.text".into(),
    }];
    c.sources = src(&[wiki::VIBE_FAILSAFE, wiki::ERR_TABLE]);
    c
}

#[allow(clippy::needless_late_init)]
fn ekf_variance(ctx: &Ctx) -> Check {
    const ID: &str = "ekf.variance";
    let log = ctx.log;
    let thresh = ctx.param("FS_EKF_THRESH", Some(0.8));
    let (limit, origin) = thresh.unwrap_or((0.8, "param_default"));
    // wiki: FS_EKF_THRESH = 0 disables the failsafe; still compare with the default
    let (limit, origin) = if limit <= 0.0 {
        (0.8, "param_default")
    } else {
        (limit, origin)
    };
    let flight = ctx.flight();
    // (label, velocity, position, compass)
    let mut cores: Vec<(String, Series, Series, Series)> = Vec::new();
    let evidence: Vec<String>;
    match log.kind {
        LogKind::DataFlash => {
            let (name, t) = match (ctx.table("XKF4"), ctx.table("NKF4")) {
                (Some(t), _) => ("XKF4", t),
                (None, Some(t)) => ("NKF4", t),
                _ => return na(ID, "EKF variances (XKF4 / NKF4)", &[wiki::EKF_FAILSAFE]),
            };
            let insts = if t.column("C").is_some() {
                distinct(t, "C")
            } else {
                vec![0]
            };
            for inst in insts {
                let rows = instance_rows(t, "C", inst);
                let s = |f: &str| Series::from_table(log, t, f, Some(&rows));
                if let (Some(v), Some(p), Some(m)) = (s("SV"), s("SP"), s("SM")) {
                    cores.push((format!("core{inst}"), v, p, m));
                }
            }
            evidence = vec![
                format!("{name}.SV"),
                format!("{name}.SP"),
                format!("{name}.SM"),
                "ERR.Subsys=16,17".into(),
            ];
        }
        LogKind::Tlog => {
            let Some(t) = ctx.table("EKF_STATUS_REPORT") else {
                return na(ID, "EKF_STATUS_REPORT", &[wiki::EKF_FAILSAFE]);
            };
            let s = |f: &str| Series::from_table(log, t, f, None);
            if let (Some(v), Some(p), Some(m)) = (
                s("velocity_variance"),
                s("pos_horiz_variance"),
                s("compass_variance"),
            ) {
                cores.push(("primary".into(), v, p, m));
            }
            evidence = vec![
                "EKF_STATUS_REPORT.velocity_variance".into(),
                "EKF_STATUS_REPORT.pos_horiz_variance".into(),
                "EKF_STATUS_REPORT.compass_variance".into(),
            ];
        }
    }
    if cores.is_empty() {
        return na(ID, "EKF variances", &[wiki::EKF_FAILSAFE]);
    }
    let mut measured = Map::new();
    let mut ranges = Vec::new();
    let mut two_over = false;
    let mut one_over = false;
    for (label, v, p, m) in &cores {
        let (v, p, m) = (v.within(&flight), p.within(&flight), m.within(&flight));
        measured.insert(
            label.clone(),
            json!({ "velocity_max": opt(v.max()), "position_max": opt(p.max()), "compass_max": opt(m.max()) }),
        );
        // count of variances over the limit at each velocity sample time
        let n = v.len().min(p.len()).min(m.len());
        let over = Series {
            t: v.t[..n].to_vec(),
            v: (0..n)
                .map(|i| {
                    [v.v[i], p.v[i], m.v[i]]
                        .iter()
                        .filter(|&&x| x > limit)
                        .count() as f64
                })
                .collect(),
        };
        for (a, b) in over.runs(|c| c >= 2.0, 0) {
            if b - a >= 1_000_000 {
                two_over = true;
                ranges.push(ctx.range(
                    a,
                    b,
                    format!("{label}: two or more variances above {limit} for 1 s or longer"),
                ));
            }
        }
        for (a, b) in over.runs(|c| c >= 1.0, 0) {
            if b - a >= 1_000_000 {
                one_over = true;
                ranges.push(ctx.range(
                    a,
                    b,
                    format!("{label}: a variance above {limit} for 1 s or longer"),
                ));
            }
        }
    }
    let fs_err: Vec<(u64, u64, i64)> = ctx.err_ranges(17, &[1], 0);
    let check_err: Vec<(u64, u64, i64)> = ctx.err_ranges(16, &[2], 0);
    for (a, b, _) in &fs_err {
        ranges.push(ctx.range(*a, *b, "ERR EKF Failsafe: Failsafe Triggered"));
    }
    for (a, b, _) in &check_err {
        ranges.push(ctx.range(
            *a,
            *b,
            "ERR EKF Check: Bad Variance (position estimate bad)",
        ));
    }
    let texts = if log.kind == LogKind::Tlog {
        ctx.text_ranges(
            &|x: &str| x.starts_with("EKF Failsafe") && !x.contains("Cleared"),
            &|x: &str| x.starts_with("EKF Failsafe Cleared"),
        )
    } else {
        Vec::new()
    };
    for (a, b, x) in &texts {
        ranges.push(ctx.range(*a, *b, x.clone()));
    }
    measured.insert(
        "ekf_failsafe_events".into(),
        json!(fs_err.len() + texts.len()),
    );
    let (status, summary) = if !fs_err.is_empty() || !texts.is_empty() {
        (
            Status::Fail,
            format!(
                "The EKF failsafe triggered {} time(s).",
                fs_err.len() + texts.len()
            ),
        )
    } else if two_over {
        (Status::Fail, format!("Two or more EKF variances stayed above FS_EKF_THRESH {limit} for 1 s or longer, the wiki's EKF failsafe condition."))
    } else if !check_err.is_empty() || one_over {
        (Status::Warn, format!("An EKF variance stayed above FS_EKF_THRESH {limit} for 1 s or longer (one variance alone does not trigger the failsafe)."))
    } else {
        (
            Status::Pass,
            format!("EKF variances stayed below FS_EKF_THRESH {limit}."),
        )
    };
    ranges.sort_by_key(|r| r.start_us);
    let mut c = check(ID, status, summary);
    c.thresholds = vec![
        th("FS_EKF_THRESH", limit, "", origin),
        th("duration", 1.0, "s", "wiki"),
    ];
    c.measured = measured;
    c.ranges = ranges;
    c.evidence = evidence;
    c.sources = src(&[wiki::EKF_FAILSAFE, wiki::EKF_DEFAULT, wiki::ERR_TABLE]);
    c
}

/// GPS samples with a 3D fix while flying, primary receiver: (hdop, nsats).
fn gps_series(ctx: &Ctx) -> Option<(Series, Series, Vec<String>)> {
    let log = ctx.log;
    let flight = ctx.flight();
    match log.kind {
        LogKind::DataFlash => {
            let t = ctx.table("GPS")?;
            let mut rows = instance_rows(t, "I", 0);
            if let Some(st) = t.column("Status") {
                rows.retain(|&i| st.data.get_i64(i).unwrap_or(0) >= 3);
            }
            let h = Series::from_table(log, t, "HDop", Some(&rows))?.within(&flight);
            let n = Series::from_table(log, t, "NSats", Some(&rows))?.within(&flight);
            Some((
                h,
                n,
                vec!["GPS.HDop".into(), "GPS.NSats".into(), "GPS.Status".into()],
            ))
        }
        LogKind::Tlog => {
            let t = ctx.table("GPS_RAW_INT")?;
            let sys = vehicle_sys_id(log);
            let (sid, fix) = (t.column("sys_id")?, t.column("fix_type")?);
            let rows: Vec<usize> = (0..t.len())
                .filter(|&i| {
                    (sys.is_none() || sid.data.get_i64(i) == sys)
                        && fix.data.get_i64(i).unwrap_or(0) >= 3
                })
                .collect();
            // eph is HDOP * 100; 65535 = unknown
            let h = Series::from_table(log, t, "eph", Some(&rows))?
                .filter(|v| v < 65535.0)
                .within(&flight);
            let h = Series {
                t: h.t,
                v: h.v.iter().map(|v| v / 100.0).collect(),
            };
            let n = Series::from_table(log, t, "satellites_visible", Some(&rows))?
                .filter(|v| v < 255.0)
                .within(&flight);
            Some((
                h,
                n,
                vec![
                    "GPS_RAW_INT.eph".into(),
                    "GPS_RAW_INT.satellites_visible".into(),
                    "GPS_RAW_INT.fix_type".into(),
                ],
            ))
        }
    }
}

fn gps_cites(ctx: &Ctx) -> Vec<Cite> {
    match ctx.log.kind {
        LogKind::DataFlash => vec![wiki::GPS_DF],
        LogKind::Tlog => vec![wiki::GPS_TLOG],
    }
}

fn gps_hdop(ctx: &Ctx) -> Check {
    const ID: &str = "gps.hdop";
    let Some((h, _, evidence)) = gps_series(ctx).filter(|(h, _, _)| !h.is_empty()) else {
        return na(ID, "GPS HDOP with a 3D fix while armed", &gps_cites(ctx));
    };
    let max = h.max().unwrap_or(0.0);
    let over2 = h.runs(|v| v > 2.0, ctx.opts.merge_gap_us);
    let (status, summary) = if max > 2.0 {
        (Status::Warn, format!("HDOP rose above 2.0 (max {max:.2}); the wiki says values over 2.0 could indicate the GPS positions are not good."))
    } else if max >= 1.5 {
        (
            Status::Info,
            format!("HDOP stayed at or below 2.0 but reached {max:.2} (below 1.5 is very good)."),
        )
    } else {
        (
            Status::Pass,
            format!("HDOP stayed below 1.5 (max {max:.2})."),
        )
    };
    let mut c = check(ID, status, summary);
    c.thresholds = vec![
        th("very_good_below", 1.5, "", "wiki"),
        th("warn_above", 2.0, "", "wiki"),
    ];
    c.measured.insert("max".into(), num(max));
    c.measured.insert("p95".into(), opt(h.percentile(0.95)));
    c.measured
        .insert("seconds_above_2".into(), num(total_s(&over2)));
    c.ranges = over2
        .iter()
        .map(|&(a, b)| ctx.range(a, b, "HDOP above 2.0"))
        .collect();
    c.evidence = evidence;
    c.sources = src(&gps_cites(ctx));
    c
}

fn gps_satellites(ctx: &Ctx) -> Check {
    const ID: &str = "gps.satellites";
    let Some((_, n, evidence)) = gps_series(ctx).filter(|(_, n, _)| !n.is_empty()) else {
        return na(
            ID,
            "GPS satellite counts with a 3D fix while armed",
            &gps_cites(ctx),
        );
    };
    let min = n.min().unwrap_or(0.0);
    let below = n.runs(|v| v < 12.0, ctx.opts.merge_gap_us);
    let (status, summary) = if min < 12.0 {
        (
            Status::Warn,
            format!(
                "Satellite count fell to {min:.0}, below the 12 the wiki calls bad to fall below."
            ),
        )
    } else {
        (
            Status::Pass,
            format!("Satellite count stayed at 12 or more (min {min:.0})."),
        )
    };
    let mut c = check(ID, status, summary);
    c.thresholds = vec![th("warn_below", 12.0, "satellites", "wiki")];
    c.measured.insert("min".into(), num(min));
    c.measured.insert("max".into(), opt(n.max()));
    c.measured
        .insert("seconds_below_12".into(), num(total_s(&below)));
    c.ranges = below
        .iter()
        .map(|&(a, b)| ctx.range(a, b, "fewer than 12 satellites"))
        .collect();
    c.evidence = evidence;
    c.sources = src(&gps_cites(ctx));
    c
}

fn gps_glitch(ctx: &Ctx) -> Check {
    const ID: &str = "gps.glitch";
    let ranges: Vec<(u64, u64, String)> = match ctx.log.kind {
        LogKind::DataFlash => ctx
            .err_ranges(11, &[2], 0)
            .into_iter()
            .map(|(a, b, _)| (a, b, "ERR GPS: GPS Glitch occurred".to_string()))
            .collect(),
        LogKind::Tlog => ctx.text_ranges(&|x: &str| x.starts_with("GPS Glitch"), &|x: &str| {
            x == "Glitch cleared"
        }),
    };
    let mut c = if ranges.is_empty() {
        check(ID, Status::Pass, "No GPS glitch was logged.".into())
    } else {
        check(
            ID,
            Status::Fail,
            format!("The firmware logged {} GPS glitch(es).", ranges.len()),
        )
    };
    c.measured.insert("glitches".into(), json!(ranges.len()));
    c.ranges = ranges
        .iter()
        .map(|(a, b, d)| ctx.range(*a, *b, d.clone()))
        .collect();
    c.evidence = vec![match ctx.log.kind {
        LogKind::DataFlash => "ERR.Subsys=11".into(),
        LogKind::Tlog => "STATUSTEXT.text".into(),
    }];
    c.sources = src(&[wiki::ERR_TABLE, gps_cites(ctx)[0]]);
    c
}

fn battery_voltage(ctx: &Ctx) -> Check {
    const ID: &str = "battery.voltage";
    let log = ctx.log;
    let flight = ctx.flight();
    let low = ctx.param("BATT_LOW_VOLT", Some(10.5));
    let crt = ctx.param("BATT_CRT_VOLT", None);
    let timer = ctx
        .param("BATT_LOW_TIMER", Some(10.0))
        .map_or(10.0, |v| v.0);
    let src_sel = ctx.param("BATT_FS_VOLTSRC", Some(0.0)).map_or(0.0, |v| v.0);
    let mut measured = Map::new();
    let mut evidence = Vec::new();
    let volt: Series;
    match log.kind {
        LogKind::DataFlash => {
            let Some(t) = ctx.table("BAT").or_else(|| ctx.table("CURR")) else {
                return na(ID, "battery messages (BAT)", &[wiki::BATTERY]);
            };
            let inst = if t.column("Inst").is_some() {
                "Inst"
            } else {
                "Instance"
            };
            let rows = instance_rows(t, inst, 0);
            let raw = Series::from_table(log, t, "Volt", Some(&rows))
                .unwrap_or_default()
                .within(&flight);
            let rest = Series::from_table(log, t, "VoltR", Some(&rows)).map(|s| s.within(&flight));
            evidence.push(format!("{}.Volt", t.name));
            if let Some(r) = &rest {
                evidence.push(format!("{}.VoltR", t.name));
                // sag = resting estimate minus measured
                let n = r.len().min(raw.len());
                let sag = (0..n).map(|i| r.v[i] - raw.v[i]).fold(f64::NAN, f64::max);
                measured.insert("max_sag_v".into(), num(sag));
                measured.insert("min_resting_v".into(), opt(r.min()));
            }
            measured.insert("min_v".into(), opt(raw.min()));
            volt = if src_sel >= 1.0 {
                rest.unwrap_or(raw)
            } else {
                raw
            };
        }
        LogKind::Tlog => {
            let Some(t) = ctx.table("SYS_STATUS") else {
                return na(ID, "SYS_STATUS", &[wiki::BATTERY]);
            };
            let sys = vehicle_sys_id(log);
            let rows: Vec<usize> = match (sys, t.column("sys_id")) {
                (Some(s), Some(c)) => (0..t.len())
                    .filter(|&i| c.data.get_i64(i) == Some(s))
                    .collect(),
                _ => (0..t.len()).collect(),
            };
            // voltage_battery in mV; UINT16_MAX = not sent
            let s = Series::from_table(log, t, "voltage_battery", Some(&rows))
                .unwrap_or_default()
                .filter(|v| v < 65535.0);
            volt = Series {
                t: s.t,
                v: s.v.iter().map(|v| v / 1000.0).collect(),
            }
            .within(&flight);
            evidence.push("SYS_STATUS.voltage_battery".into());
            measured.insert("min_v".into(), opt(volt.min()));
        }
    }
    let mut ranges = Vec::new();
    let fs = ctx.err_ranges(6, &[1, 2, 3], 0);
    for (a, b, code) in &fs {
        ranges.push(ctx.range(
            *a,
            *b,
            format!("ERR Battery Failsafe: Failsafe Triggered (code {code})"),
        ));
    }
    let fs_text = if log.kind == LogKind::Tlog {
        ctx.text_ranges(
            &|x: &str| {
                x.starts_with("Battery Failsafe") || x.starts_with("Radio + Battery Failsafe")
            },
            &|_| false,
        )
    } else {
        Vec::new()
    };
    for (a, b, x) in &fs_text {
        ranges.push(ctx.range(*a, *b, x.clone()));
    }
    if fs.is_empty() {
        evidence.push("ERR.Subsys=6".into());
    }
    let mut thresholds = vec![th(
        "BATT_LOW_TIMER",
        timer,
        "s",
        if log.param("BATT_LOW_TIMER").is_some() {
            "param"
        } else {
            "param_default"
        },
    )];
    let mut long_low = false;
    let mut any_low = false;
    if volt.is_empty() && fs.is_empty() && fs_text.is_empty() {
        return na(ID, "battery voltage while armed", &[wiki::BATTERY]);
    }
    if let Some((lv, origin)) = low.filter(|(v, _)| *v > 0.0) {
        thresholds.push(th("BATT_LOW_VOLT", lv, "V", origin));
        for (a, b) in volt.runs(|v| v < lv, 0) {
            any_low = true;
            let long = (b - a) as f64 / 1e6 > timer;
            long_low |= long;
            ranges.push(ctx.range(
                a,
                b,
                if long {
                    format!("below BATT_LOW_VOLT {lv} V for more than {timer} s")
                } else {
                    format!("below BATT_LOW_VOLT {lv} V")
                },
            ));
        }
    }
    if let Some((cv, origin)) = crt.filter(|(v, _)| *v > 0.0) {
        thresholds.push(th("BATT_CRT_VOLT", cv, "V", origin));
        for (a, b) in volt.runs(|v| v < cv, 0) {
            if (b - a) as f64 / 1e6 > timer {
                long_low = true;
            }
            ranges.push(ctx.range(a, b, format!("below BATT_CRT_VOLT {cv} V")));
        }
    }
    measured.insert(
        "voltage_source".into(),
        json!(if src_sel >= 1.0 && log.kind == LogKind::DataFlash {
            "sag_compensated"
        } else {
            "raw"
        }),
    );
    let (status, summary) = if !fs.is_empty() || !fs_text.is_empty() {
        (
            Status::Fail,
            format!(
                "The battery failsafe triggered {} time(s).",
                fs.len() + fs_text.len()
            ),
        )
    } else if long_low {
        (
            Status::Fail,
            format!("Battery voltage stayed below the failsafe threshold for more than {timer} s."),
        )
    } else if any_low {
        (
            Status::Warn,
            "Battery voltage dipped below BATT_LOW_VOLT, for less than the failsafe timer.".into(),
        )
    } else if low.is_none_or(|(v, _)| v <= 0.0) {
        (Status::Info, "The voltage failsafe is disabled (BATT_LOW_VOLT = 0) and no battery failsafe was logged.".into())
    } else {
        (
            Status::Pass,
            "Battery voltage stayed above BATT_LOW_VOLT while armed.".into(),
        )
    };
    ranges.sort_by_key(|r| r.start_us);
    let mut c = check(ID, status, summary);
    c.thresholds = thresholds;
    c.measured = measured;
    c.ranges = ranges;
    c.evidence = evidence;
    c.sources = src(&[wiki::BATTERY, wiki::BATTERY_SAG, wiki::ERR_TABLE]);
    c
}

fn power_board_vcc(ctx: &Ctx) -> Check {
    const ID: &str = "power.board_vcc";
    let log = ctx.log;
    let flight = ctx.flight();
    let s = match log.kind {
        LogKind::DataFlash => ctx
            .table("POWR")
            .and_then(|t| Series::from_table(log, t, "Vcc", None))
            .map(|s| (s, "POWR.Vcc")),
        LogKind::Tlog => ctx
            .table("POWER_STATUS")
            .and_then(|t| Series::from_table(log, t, "Vcc", None))
            .map(|s| {
                (
                    Series {
                        t: s.t,
                        v: s.v.iter().map(|v| v / 1000.0).collect(),
                    },
                    "POWER_STATUS.Vcc",
                )
            }),
    };
    let Some((s, ev)) = s
        .map(|(s, e)| (s.within(&flight), e))
        .filter(|(s, _)| !s.is_empty())
    else {
        return na(
            ID,
            "board voltage (POWR / POWER_STATUS) while armed",
            &[wiki::BOARD_VCC],
        );
    };
    let (lo, hi) = (s.min().unwrap_or(0.0), s.max().unwrap_or(0.0));
    let var = hi - lo;
    let (status, summary) = if var > 0.15 + 1e-9 {
        (Status::Warn, format!("Board voltage varied by {var:.2} V ({lo:.2} to {hi:.2}); the wiki calls 0.10 to 0.15 V normal."))
    } else {
        (
            Status::Pass,
            format!("Board voltage varied by {var:.2} V, within the 0.15 V the wiki calls normal."),
        )
    };
    let mut c = check(ID, status, summary);
    c.thresholds = vec![th("variation_warn_above", 0.15, "V", "wiki")];
    c.measured.insert("min_v".into(), num(lo));
    c.measured.insert("max_v".into(), num(hi));
    c.measured.insert("variation_v".into(), num(var));
    c.evidence = vec![ev.into()];
    c.sources = src(&[wiki::BOARD_VCC]);
    c
}

fn power_log_end(ctx: &Ctx) -> Check {
    const ID: &str = "power.log_ends_armed";
    if ctx.armed_source == "none" {
        return na(ID, "arming state", &[wiki::BROWNOUT]);
    }
    let ends_armed = ctx.armed.last().is_some_and(|&(_, e)| e >= ctx.end_us);
    let mut c = if ends_armed && ctx.log.kind == LogKind::DataFlash {
        check(ID, Status::Warn, "The log ends while the vehicle is armed. The wiki notes brown-outs show up as logs suddenly ending in the air; check altitude at the end.".into())
    } else if ends_armed {
        check(ID, Status::Info, "The telemetry log ends while the vehicle is armed (the ground station may simply have stopped recording).".into())
    } else {
        check(
            ID,
            Status::Pass,
            "The vehicle is disarmed before the log ends.".into(),
        )
    };
    if ends_armed {
        c.ranges
            .push(ctx.range(ctx.end_us, ctx.end_us, "log ends armed"));
    }
    c.evidence = vec![ctx.armed_source.into()];
    c.sources = src(&[wiki::BROWNOUT]);
    c
}

fn rc_failsafe(ctx: &Ctx) -> Check {
    const ID: &str = "rc.failsafe";
    let (fs, late): (Vec<TextRange>, Vec<TextRange>) = match ctx.log.kind {
        LogKind::DataFlash => (
            ctx.err_ranges(5, &[1], 0)
                .into_iter()
                .map(|(a, b, _)| (a, b, "ERR Radio Failsafe: Failsafe Triggered".to_string()))
                .collect(),
            ctx.err_ranges(2, &[2], 0)
                .into_iter()
                .map(|(a, b, _)| (a, b, "ERR Radio: Late Frame".to_string()))
                .collect(),
        ),
        LogKind::Tlog => (
            ctx.text_ranges(
                &|x: &str| {
                    (x.starts_with("Radio Failsafe") || x.starts_with("Radio + Battery Failsafe"))
                        && !x.contains("Cleared")
                },
                &|x: &str| x.starts_with("Radio Failsafe Cleared"),
            ),
            Vec::new(),
        ),
    };
    let mut c = if !fs.is_empty() {
        check(
            ID,
            Status::Fail,
            format!(
                "The radio (RC) failsafe triggered {} time(s), {:.1} s in total.",
                fs.len(),
                total_s(&fs.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>())
            ),
        )
    } else if !late.is_empty() {
        check(ID, Status::Warn, format!("The receiver sent no updates for two seconds {} time(s) (ERR Radio Late Frame) without a radio failsafe.", late.len()))
    } else {
        check(
            ID,
            Status::Pass,
            "No radio failsafe or late RC frames were logged.".into(),
        )
    };
    if let Some((v, origin)) = ctx.param("RC_FS_TIMEOUT", Some(1.0)) {
        c.thresholds.push(th("RC_FS_TIMEOUT", v, "s", origin));
    }
    c.measured.insert("failsafes".into(), json!(fs.len()));
    c.measured.insert("late_frames".into(), json!(late.len()));
    c.ranges = fs
        .iter()
        .chain(late.iter())
        .map(|(a, b, d)| ctx.range(*a, *b, d.clone()))
        .collect();
    c.ranges.sort_by_key(|r| r.start_us);
    c.evidence = vec![match ctx.log.kind {
        LogKind::DataFlash => "ERR.Subsys=2,5".into(),
        LogKind::Tlog => "STATUSTEXT.text".into(),
    }];
    c.sources = src(&[wiki::RADIO_FAILSAFE, wiki::ERR_TABLE]);
    c
}

fn gcs_failsafe(ctx: &Ctx) -> Check {
    const ID: &str = "link.gcs_failsafe";
    let log = ctx.log;
    let timeout = ctx.param("FS_GCS_TIMEOUT", Some(5.0));
    let fs: Vec<(u64, u64, String)> = match log.kind {
        LogKind::DataFlash => ctx
            .err_ranges(8, &[1], 0)
            .into_iter()
            .map(|(a, b, _)| (a, b, "ERR GCS Failsafe: Failsafe Triggered".to_string()))
            .collect(),
        LogKind::Tlog => ctx.text_ranges(
            &|x: &str| x.starts_with("GCS Failsafe") && !x.contains("Cleared"),
            &|x: &str| x.starts_with("GCS Failsafe Cleared"),
        ),
    };
    // tlog: gaps in the vehicle's heartbeats while armed, as the ground station saw them
    let mut gaps: Vec<(u64, u64)> = Vec::new();
    if log.kind == LogKind::Tlog {
        if let (Some(hb), Some(sys), Some((to, _))) =
            (ctx.table("HEARTBEAT"), vehicle_sys_id(log), timeout)
        {
            if let (Some(tc), Some(sid)) = (hb.column("timestamp_us"), hb.column("sys_id")) {
                let ts: Vec<u64> = (0..hb.len())
                    .filter(|&i| sid.data.get_i64(i) == Some(sys))
                    .filter_map(|i| tc.data.get_f64(i).map(|v| v as u64))
                    .collect();
                let flight = ctx.flight();
                for w in ts.windows(2) {
                    if (w[1].saturating_sub(w[0])) as f64 / 1e6 > to
                        && super::series::in_ranges(&flight, w[0])
                    {
                        gaps.push((w[0], w[1]));
                    }
                }
            }
        }
    }
    let mut c = if !fs.is_empty() {
        check(
            ID,
            Status::Fail,
            format!("The GCS failsafe triggered {} time(s).", fs.len()),
        )
    } else if !gaps.is_empty() {
        check(ID, Status::Warn, format!("The vehicle's heartbeat was missing from the telemetry log for longer than FS_GCS_TIMEOUT {} time(s) while armed.", gaps.len()))
    } else {
        check(ID, Status::Pass, "No GCS failsafe was logged.".into())
    };
    if let Some((v, origin)) = timeout {
        c.thresholds.push(th("FS_GCS_TIMEOUT", v, "s", origin));
    }
    c.measured.insert("failsafes".into(), json!(fs.len()));
    if log.kind == LogKind::Tlog {
        c.measured
            .insert("heartbeat_gaps".into(), json!(gaps.len()));
    }
    c.ranges = fs
        .iter()
        .map(|(a, b, d)| ctx.range(*a, *b, d.clone()))
        .collect();
    c.ranges.extend(
        gaps.iter()
            .map(|&(a, b)| ctx.range(a, b, "no vehicle heartbeat received")),
    );
    c.ranges.sort_by_key(|r| r.start_us);
    c.evidence = vec![match log.kind {
        LogKind::DataFlash => "ERR.Subsys=8".into(),
        LogKind::Tlog => "STATUSTEXT.text, HEARTBEAT.timestamp_us".into(),
    }];
    c.sources = src(&[wiki::GCS_FAILSAFE, wiki::ERR_TABLE]);
    c
}

/// Flight mode names (pymavlink's mode mappings).
pub(crate) fn mode_name(vehicle: Option<&str>, n: i64) -> Option<&'static str> {
    let copter: &[(i64, &str)] = &[
        (0, "STABILIZE"),
        (1, "ACRO"),
        (2, "ALT_HOLD"),
        (3, "AUTO"),
        (4, "GUIDED"),
        (5, "LOITER"),
        (6, "RTL"),
        (7, "CIRCLE"),
        (9, "LAND"),
        (11, "DRIFT"),
        (13, "SPORT"),
        (14, "FLIP"),
        (15, "AUTOTUNE"),
        (16, "POSHOLD"),
        (17, "BRAKE"),
        (18, "THROW"),
        (19, "AVOID_ADSB"),
        (20, "GUIDED_NOGPS"),
        (21, "SMART_RTL"),
        (22, "FLOWHOLD"),
        (23, "FOLLOW"),
        (24, "ZIGZAG"),
        (25, "SYSTEMID"),
        (26, "AUTOROTATE"),
        (27, "AUTO_RTL"),
        (28, "TURTLE"),
    ];
    let plane: &[(i64, &str)] = &[
        (0, "MANUAL"),
        (1, "CIRCLE"),
        (2, "STABILIZE"),
        (3, "TRAINING"),
        (4, "ACRO"),
        (5, "FBWA"),
        (6, "FBWB"),
        (7, "CRUISE"),
        (8, "AUTOTUNE"),
        (10, "AUTO"),
        (11, "RTL"),
        (12, "LOITER"),
        (13, "TAKEOFF"),
        (14, "AVOID_ADSB"),
        (15, "GUIDED"),
        (16, "INITIALISING"),
        (17, "QSTABILIZE"),
        (18, "QHOVER"),
        (19, "QLOITER"),
        (20, "QLAND"),
        (21, "QRTL"),
        (22, "QAUTOTUNE"),
        (23, "QACRO"),
        (24, "THERMAL"),
        (25, "LOITERALTQLAND"),
        (26, "AUTOLAND"),
    ];
    let rover: &[(i64, &str)] = &[
        (0, "MANUAL"),
        (1, "ACRO"),
        (3, "STEERING"),
        (4, "HOLD"),
        (5, "LOITER"),
        (6, "FOLLOW"),
        (7, "SIMPLE"),
        (8, "DOCK"),
        (9, "CIRCLE"),
        (10, "AUTO"),
        (11, "RTL"),
        (12, "SMART_RTL"),
        (15, "GUIDED"),
        (16, "INITIALISING"),
    ];
    let table = match vehicle? {
        "copter" | "heli" => copter,
        "plane" => plane,
        "rover" => rover,
        _ => return None,
    };
    table.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

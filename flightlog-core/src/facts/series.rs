//! Small time-series helpers for the checks.

use crate::{Log, LogKind, Table};

/// Values of one field with their timestamps (log time base, microseconds).
#[derive(Debug, Clone, Default)]
pub(crate) struct Series {
    pub t: Vec<u64>,
    pub v: Vec<f64>,
}

impl Series {
    pub fn from_table(
        log: &Log,
        table: &Table,
        field: &str,
        rows: Option<&[usize]>,
    ) -> Option<Series> {
        let tc = table.column(log.time_column())?;
        let vc = table.column(field)?;
        let mut s = Series::default();
        let mut push = |i: usize| {
            if let (Some(t), Some(v)) = (tc.data.get_f64(i), vc.data.get_f64(i)) {
                s.t.push(t as u64);
                s.v.push(v);
            }
        };
        match rows {
            Some(r) => r.iter().for_each(|&i| push(i)),
            None => (0..table.len()).for_each(push),
        }
        Some(s)
    }

    pub fn len(&self) -> usize {
        self.v.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v.is_empty()
    }

    /// Only the samples inside the ranges.
    pub fn within(&self, ranges: &[(u64, u64)]) -> Series {
        let mut out = Series::default();
        for (i, &t) in self.t.iter().enumerate() {
            if in_ranges(ranges, t) {
                out.t.push(t);
                out.v.push(self.v[i]);
            }
        }
        out
    }

    pub fn filter(&self, keep: impl Fn(f64) -> bool) -> Series {
        let mut out = Series::default();
        for (i, &v) in self.v.iter().enumerate() {
            if keep(v) {
                out.t.push(self.t[i]);
                out.v.push(v);
            }
        }
        out
    }

    pub fn max(&self) -> Option<f64> {
        self.v
            .iter()
            .copied()
            .filter(|v| !v.is_nan())
            .reduce(f64::max)
    }

    pub fn min(&self) -> Option<f64> {
        self.v
            .iter()
            .copied()
            .filter(|v| !v.is_nan())
            .reduce(f64::min)
    }

    /// Nearest-rank percentile, `p` in 0..=1.
    pub fn percentile(&self, p: f64) -> Option<f64> {
        let mut v: Vec<f64> = self.v.iter().copied().filter(|v| !v.is_nan()).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
        let rank = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len());
        Some(v[rank - 1])
    }

    /// Time ranges where `pred` holds, from the first sample where it holds
    /// to the last; runs separated by less than `merge_us` are joined.
    pub fn runs(&self, pred: impl Fn(f64) -> bool, merge_us: u64) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut cur: Option<(u64, u64)> = None;
        for (i, &v) in self.v.iter().enumerate() {
            let t = self.t[i];
            if pred(v) {
                cur = Some(match cur {
                    Some((s, _)) => (s, t),
                    None => (t, t),
                });
            } else if let Some(c) = cur.take() {
                push_merged(&mut out, c, merge_us);
            }
        }
        if let Some(c) = cur {
            push_merged(&mut out, c, merge_us);
        }
        out
    }
}

fn push_merged(out: &mut Vec<(u64, u64)>, r: (u64, u64), merge_us: u64) {
    if let Some(last) = out.last_mut() {
        if r.0.saturating_sub(last.1) < merge_us {
            last.1 = r.1;
            return;
        }
    }
    out.push(r);
}

pub(crate) fn in_ranges(ranges: &[(u64, u64)], t: u64) -> bool {
    // ranges are sorted and disjoint
    match ranges.binary_search_by(|r| r.0.cmp(&t)) {
        Ok(_) => true,
        Err(0) => false,
        Err(i) => t <= ranges[i - 1].1,
    }
}

/// Total duration of the ranges in seconds.
pub(crate) fn total_s(ranges: &[(u64, u64)]) -> f64 {
    ranges.iter().fold(0.0, |a, r| a + (r.1 - r.0) as f64 / 1e6)
}

/// Row indices of a table whose instance field equals `inst`.
pub(crate) fn instance_rows(table: &Table, field: &str, inst: i64) -> Vec<usize> {
    match table.column(field) {
        Some(c) => (0..table.len())
            .filter(|&i| c.data.get_i64(i) == Some(inst))
            .collect(),
        None => (0..table.len()).collect(),
    }
}

/// Distinct values of an integer column in first-seen order.
pub(crate) fn distinct(table: &Table, field: &str) -> Vec<i64> {
    let mut out = Vec::new();
    if let Some(c) = table.column(field) {
        for i in 0..c.data.len() {
            if let Some(v) = c.data.get_i64(i) {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
        }
    }
    out
}

/// Armed time ranges and where they came from.
pub(crate) fn armed_ranges(log: &Log, end: u64) -> (Vec<(u64, u64)>, &'static str) {
    let mut changes: Vec<(u64, bool)> = Vec::new();
    let source;
    match log.kind {
        LogKind::DataFlash => {
            if let Some(arm) = log.table("ARM") {
                source = "ARM.ArmState";
                if let (Some(t), Some(s)) = (arm.column("TimeUS"), arm.column("ArmState")) {
                    for i in 0..arm.len() {
                        if let (Some(t), Some(s)) = (t.data.get_f64(i), s.data.get_i64(i)) {
                            changes.push((t as u64, s != 0));
                        }
                    }
                }
            } else if let Some(ev) = log.table("EV") {
                // AP_Logger LogEvent: 10 ARMED, 11 DISARMED
                source = "EV.Id";
                if let (Some(t), Some(id)) = (ev.column("TimeUS"), ev.column("Id")) {
                    for i in 0..ev.len() {
                        match (t.data.get_f64(i), id.data.get_i64(i)) {
                            (Some(t), Some(10)) => changes.push((t as u64, true)),
                            (Some(t), Some(11)) => changes.push((t as u64, false)),
                            _ => {}
                        }
                    }
                }
            } else {
                return (Vec::new(), "none");
            }
        }
        LogKind::Tlog => {
            let Some(hb) = log.table("HEARTBEAT") else {
                return (Vec::new(), "none");
            };
            source = "HEARTBEAT.base_mode";
            let sys = vehicle_sys_id(log);
            let (Some(t), Some(bm), Some(sid), Some(ap), Some(ty)) = (
                hb.column("timestamp_us"),
                hb.column("base_mode"),
                hb.column("sys_id"),
                hb.column("autopilot"),
                hb.column("type"),
            ) else {
                return (Vec::new(), "none");
            };
            let mut last: Option<bool> = None;
            for i in 0..hb.len() {
                // vehicle heartbeats only: not a GCS (type 6), autopilot not INVALID (8)
                if sid.data.get_i64(i) != sys
                    || ty.data.get_i64(i) == Some(6)
                    || ap.data.get_i64(i) == Some(8)
                {
                    continue;
                }
                let armed = bm.data.get_i64(i).unwrap_or(0) & 128 != 0;
                if last != Some(armed) {
                    changes.push((t.data.get_f64(i).unwrap_or(0.0) as u64, armed));
                    last = Some(armed);
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut start: Option<u64> = None;
    for (t, armed) in changes {
        match (armed, start) {
            (true, None) => start = Some(t),
            (false, Some(s)) => {
                out.push((s, t));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, end.max(s)));
    }
    (out, source)
}

/// System id of the vehicle in a tlog: the most common sender of non-GCS heartbeats.
pub(crate) fn vehicle_sys_id(log: &Log) -> Option<i64> {
    let hb = log.table("HEARTBEAT")?;
    let (sid, ty, ap) = (
        hb.column("sys_id")?,
        hb.column("type")?,
        hb.column("autopilot")?,
    );
    let mut counts: std::collections::BTreeMap<i64, usize> = Default::default();
    for i in 0..hb.len() {
        if ty.data.get_i64(i) == Some(6) || ap.data.get_i64(i) == Some(8) {
            continue;
        }
        *counts.entry(sid.data.get_i64(i)?).or_insert(0) += 1;
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_merge() {
        let s = Series {
            t: vec![0, 1, 2, 3, 10, 11, 30],
            v: vec![5.0, 5.0, 0.0, 5.0, 5.0, 0.0, 5.0],
        };
        assert_eq!(s.runs(|v| v > 1.0, 3), vec![(0, 10), (30, 30)]);
        assert_eq!(s.runs(|v| v > 1.0, 0), vec![(0, 1), (3, 10), (30, 30)]);
    }

    #[test]
    fn percentile_nearest_rank() {
        let s = Series {
            t: (0..20).collect(),
            v: (1..=20).map(|x| x as f64).collect(),
        };
        assert_eq!(s.percentile(0.95), Some(19.0));
        assert_eq!(s.percentile(1.0), Some(20.0));
        assert_eq!(s.percentile(0.0), Some(1.0));
    }

    #[test]
    fn ranges_lookup() {
        let r = [(10, 20), (30, 40)];
        assert!(!in_ranges(&r, 5));
        assert!(in_ranges(&r, 10));
        assert!(in_ranges(&r, 20));
        assert!(!in_ranges(&r, 25));
        assert!(in_ranges(&r, 40));
        assert!(!in_ranges(&r, 41));
    }
}

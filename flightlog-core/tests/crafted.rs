//! flight-facts on small DataFlash logs built in the test: damaged
//! timestamps, vehicles other than Copter, unusable parameters and EKF lanes.

#![cfg(feature = "facts")]

use flightlog_core::facts::{self, FlightFacts, Status};
use flightlog_core::{Log, LogKind, ReadOptions};

/// A DataFlash log written message by message.
struct Df(Vec<u8>);

fn pad<const N: usize>(s: &str) -> [u8; N] {
    let mut b = [0u8; N];
    b[..s.len()].copy_from_slice(s.as_bytes());
    b
}

impl Df {
    fn new() -> Df {
        let mut d = Df(Vec::new());
        d.fmt(128, "FMT", "BBnNZ", "Type,Length,Name,Format,Columns", 89);
        d
    }

    fn fmt(&mut self, id: u8, name: &str, format: &str, cols: &str, len: u8) {
        self.0.extend_from_slice(&[0xA3, 0x95, 0x80, id, len]);
        self.0.extend_from_slice(&pad::<4>(name));
        self.0.extend_from_slice(&pad::<16>(format));
        self.0.extend_from_slice(&pad::<64>(cols));
    }

    fn msg(&mut self, id: u8, body: &[u8]) {
        self.0.extend_from_slice(&[0xA3, 0x95, id]);
        self.0.extend_from_slice(body);
    }

    /// FMT for VER (vehicle), ARM, VIBE, XKF4 (with core and primary
    /// index), BAT and PARM.
    fn standard(bu: u8) -> Df {
        let mut d = Df::new();
        d.fmt(10, "VER", "QB", "TimeUS,BU", 3 + 9);
        d.fmt(11, "ARM", "QB", "TimeUS,ArmState", 3 + 9);
        d.fmt(12, "VIBE", "Qfff", "TimeUS,VibeX,VibeY,VibeZ", 3 + 20);
        d.fmt(13, "XKF4", "QBfffB", "TimeUS,C,SV,SP,SM,PI", 3 + 22);
        d.fmt(14, "BAT", "QBf", "TimeUS,Inst,Volt", 3 + 13);
        d.fmt(15, "PARM", "QNf", "TimeUS,Name,Value", 3 + 28);
        d.ver(bu);
        d
    }

    fn ver(&mut self, bu: u8) {
        let mut b = 500_000u64.to_le_bytes().to_vec();
        b.push(bu);
        self.msg(10, &b);
    }

    fn arm(&mut self, t: u64, on: bool) {
        let mut b = t.to_le_bytes().to_vec();
        b.push(on as u8);
        self.msg(11, &b);
    }

    fn vibe(&mut self, t: u64, v: f32) {
        let mut b = t.to_le_bytes().to_vec();
        for _ in 0..3 {
            b.extend_from_slice(&v.to_le_bytes());
        }
        self.msg(12, &b);
    }

    fn xkf4(&mut self, t: u64, core: u8, var: f32, primary: u8) {
        let mut b = t.to_le_bytes().to_vec();
        b.push(core);
        for _ in 0..3 {
            b.extend_from_slice(&var.to_le_bytes());
        }
        b.push(primary);
        self.msg(13, &b);
    }

    fn bat(&mut self, t: u64, volt: f32) {
        let mut b = t.to_le_bytes().to_vec();
        b.push(0);
        b.extend_from_slice(&volt.to_le_bytes());
        self.msg(14, &b);
    }

    fn parm(&mut self, name: &str, v: f32) {
        let mut b = 600_000u64.to_le_bytes().to_vec();
        b.extend_from_slice(&pad::<16>(name));
        b.extend_from_slice(&v.to_le_bytes());
        self.msg(15, &b);
    }

    /// Armed from 1 s to 21 s; samples every 0.1 s. `core1_var` and
    /// `core0_var` are the EKF variances of each core, `primary` the logged
    /// primary index, `volt` the battery voltage.
    fn flight(&mut self, core0_var: f32, core1_var: f32, primary: u8, volt: f32) {
        self.arm(1_000_000, true);
        for i in 0..200u64 {
            let t = 1_000_000 + i * 100_000;
            self.vibe(t, 5.0);
            self.xkf4(t, 0, core0_var, primary);
            self.xkf4(t, 1, core1_var, primary);
            self.bat(t, volt);
        }
        self.arm(21_000_000, false);
        for i in 0..10u64 {
            self.vibe(21_100_000 + i * 100_000, 1.0);
        }
    }

    fn facts(&self) -> (Log, FlightFacts) {
        let log = Log::read(&self.0, &ReadOptions::default()).unwrap();
        let doc = facts::run(&log, &facts::Options::default());
        (log, doc)
    }
}

fn check<'a>(doc: &'a FlightFacts, id: &str) -> &'a facts::Check {
    doc.check(id).unwrap()
}

#[test]
fn one_damaged_timestamp_does_not_stretch_the_log() {
    let mut d = Df::standard(2);
    d.flight(0.1, 0.1, 0, 12.0);
    // two VIBE rows with damaged TimeUS: one far in the future, one near zero
    d.vibe(1 << 62, 5.0);
    d.vibe(5, 5.0);
    let (log, doc) = d.facts();
    assert_eq!(log.time_range_us(), Some((500_000, 22_000_000)));
    assert_eq!(log.time_outliers(), 2);
    assert_eq!(doc.integrity.time_outliers, 2);
    assert_eq!(doc.log.duration_s, 21.5);
    assert_eq!(doc.armed.total_s, 20.0);
    assert_eq!(check(&doc, "power.log_ends_armed").status, Status::Pass);
    for c in &doc.checks {
        for r in &c.ranges {
            assert!(r.end_s <= 21.5, "{} {:?}", c.id, r);
        }
    }
}

#[test]
fn disarm_as_the_last_message_is_not_ending_armed() {
    let mut d = Df::standard(2);
    d.arm(1_000_000, true);
    for i in 0..100u64 {
        d.vibe(1_000_000 + i * 100_000, 5.0);
    }
    d.arm(11_000_000, false);
    let (_, doc) = d.facts();
    assert_eq!(doc.log.end_us, Some(11_000_000));
    assert_eq!(check(&doc, "power.log_ends_armed").status, Status::Pass);
    assert!(doc.events.iter().any(|e| e.kind == "disarmed"));

    // without the disarm the log ends armed
    let mut d = Df::standard(2);
    d.arm(1_000_000, true);
    for i in 0..100u64 {
        d.vibe(1_000_000 + i * 100_000, 5.0);
    }
    let (_, doc) = d.facts();
    assert_eq!(check(&doc, "power.log_ends_armed").status, Status::Warn);
    assert!(!doc.events.iter().any(|e| e.kind == "disarmed"));
}

#[test]
fn damaged_arming_time_is_ignored() {
    let mut d = Df::standard(2);
    d.flight(0.1, 0.1, 0, 12.0);
    d.arm(1 << 62, true);
    let (_, doc) = d.facts();
    assert_eq!(doc.armed.total_s, 20.0);
    assert_eq!(check(&doc, "power.log_ends_armed").status, Status::Pass);
}

#[test]
fn copter_log_without_params_uses_copter_defaults() {
    let mut d = Df::standard(2);
    // every variance above 0.8, 9 V on a log without BATT_LOW_VOLT
    d.flight(1.0, 1.0, 0, 9.0);
    let (_, doc) = d.facts();
    let ekf = check(&doc, "ekf.variance");
    assert_eq!(ekf.status, Status::Fail, "{}", ekf.summary);
    assert_eq!(ekf.thresholds[0].origin, "param_default");
    let bat = check(&doc, "battery.voltage");
    // below the wiki default for 20 s, but the vehicle's own threshold is unknown
    assert_eq!(bat.status, Status::Warn, "{}", bat.summary);
    assert!(bat.summary.contains("no BATT_LOW_VOLT"));
    assert!(check(&doc, "link.gcs_failsafe")
        .thresholds
        .iter()
        .any(|t| t.name == "FS_GCS_TIMEOUT" && t.origin == "param_default"));
}

#[test]
fn plane_log_gets_no_copter_defaults() {
    let mut d = Df::standard(3);
    d.flight(1.0, 1.0, 0, 7.4);
    let (_, doc) = d.facts();
    assert_eq!(doc.log.vehicle.as_deref(), Some("plane"));
    for id in ["vibration.failsafe", "ekf.variance", "battery.voltage"] {
        let c = check(&doc, id);
        assert_eq!(c.status, Status::NotApplicable, "{id}: {}", c.summary);
    }
    assert!(check(&doc, "ekf.variance").thresholds.is_empty());
    assert!(check(&doc, "ekf.variance").measured.contains_key("core0"));
    assert!(check(&doc, "rc.failsafe").thresholds.is_empty());
    assert!(check(&doc, "link.gcs_failsafe").thresholds.is_empty());
    assert!(doc.params_used.is_empty() || !doc.params_used.contains_key("FS_EKF_THRESH"));
    // vibration levels come from a common wiki page and still apply
    assert_eq!(check(&doc, "vibration.levels").status, Status::Pass);
}

#[test]
fn plane_log_with_its_own_params_is_checked() {
    let mut d = Df::standard(3);
    d.parm("BATT_LOW_VOLT", 7.0);
    d.flight(0.1, 0.1, 0, 6.8);
    let (_, doc) = d.facts();
    let bat = check(&doc, "battery.voltage");
    assert_eq!(bat.status, Status::Fail, "{}", bat.summary);
    assert_eq!(bat.thresholds[1].origin, "param");
}

#[test]
fn non_finite_params_are_not_used() {
    let mut d = Df::standard(2);
    for n in [
        "FS_EKF_THRESH",
        "RC_FS_TIMEOUT",
        "BATT_LOW_TIMER",
        "FS_GCS_TIMEOUT",
        "BATT_LOW_VOLT",
    ] {
        d.parm(n, f32::NAN);
    }
    d.flight(1.0, 1.0, 0, 12.0);
    let (_, doc) = d.facts();
    for c in &doc.checks {
        for t in &c.thresholds {
            assert!(t.value.is_finite(), "{} {}", c.id, t.name);
        }
        assert!(!c.summary.contains("NaN"), "{}", c.summary);
    }
    assert_eq!(doc.params_used["FS_EKF_THRESH"]["origin"], "param_invalid");
    assert!(doc.params_used["FS_EKF_THRESH"]["value"].is_null());
    let ekf = check(&doc, "ekf.variance");
    assert_eq!(ekf.status, Status::Fail);
    assert_eq!(ekf.thresholds[0].value, 0.8);
}

#[test]
fn ekf_failsafe_rule_reads_the_primary_core() {
    // core 1 is bad, core 0 (the primary) is fine
    let mut d = Df::standard(2);
    d.flight(0.1, 1.0, 0, 12.0);
    let (_, doc) = d.facts();
    let ekf = check(&doc, "ekf.variance");
    assert_eq!(ekf.status, Status::Warn, "{}", ekf.summary);
    assert!(ekf
        .ranges
        .iter()
        .any(|r| r.detail.contains("not the primary")));

    // the same variances with core 1 as the primary
    let mut d = Df::standard(2);
    d.flight(0.1, 1.0, 1, 12.0);
    let (_, doc) = d.facts();
    assert_eq!(check(&doc, "ekf.variance").status, Status::Fail);
}

#[test]
fn integrity_clean_has_one_meaning() {
    let mut d = Df::standard(2);
    d.flight(0.1, 0.1, 0, 12.0);
    d.0.extend_from_slice(&[0xA3, 0x95, 12, 1, 2]); // cut-off final message
    let (log, doc) = d.facts();
    assert_eq!(log.integrity.truncated_tail_bytes, 5);
    assert!(log.integrity.is_clean());
    assert_eq!(doc.integrity.clean, log.integrity.is_clean());
}

#[test]
fn detection_prefers_a_verified_tlog_frame() {
    let mut t =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mini.tlog")).unwrap();
    let want = Log::read(&t, &ReadOptions::default())
        .unwrap()
        .integrity
        .messages;
    // a first timestamp whose low bytes look like a DataFlash FMT header
    t[0] = 0xA3;
    t[1] = 0x95;
    t[2] = 0x80;
    let log = Log::read(&t, &ReadOptions::default()).unwrap();
    assert_eq!(log.kind, LogKind::Tlog);
    assert_eq!(log.integrity.messages, want);
    // text with a MAVLink magic byte at offset 8 is not a log
    assert!(Log::read(
        b"hello wo\xfdrld, this is a text file",
        &ReadOptions::default()
    )
    .is_err());
}

/// A tlog recorded in real time (no SITL speedup), written frame by frame.
struct Tl {
    bytes: Vec<u8>,
    seq: u8,
}

const T0: u64 = 1_794_000_000_000_000;

impl Tl {
    fn frame(&mut self, t_ms: u64, msg_id: u32, payload: &[u8]) {
        let mut buf = [0u8; 300];
        let n = mavlink_tlog::frame::encode(
            mavlink_tlog::frame::Version::V2,
            mavlink_tlog::frame::Header {
                seq: self.seq,
                sys_id: 1,
                comp_id: 1,
                msg_id,
            },
            payload,
            None,
            &mut buf,
        )
        .unwrap();
        self.seq = self.seq.wrapping_add(1);
        self.bytes
            .extend_from_slice(&(T0 + t_ms * 1000).to_be_bytes());
        self.bytes.extend_from_slice(&buf[..n]);
    }

    fn heartbeat(&mut self, t_ms: u64, armed: bool) {
        // custom_mode, type 2 (quadrotor), autopilot 3 (ArduPilot), base_mode
        let mut p = 0u32.to_le_bytes().to_vec();
        p.extend_from_slice(&[2, 3, if armed { 128 | 1 } else { 1 }, 4, 3]);
        self.frame(t_ms, 0, &p);
    }

    fn sys_status(&mut self, t_ms: u64, mv: u16) {
        let mut p = vec![0u8; 31];
        p[14..16].copy_from_slice(&mv.to_le_bytes());
        self.frame(t_ms, 1, &p);
    }

    fn ekf(&mut self, t_ms: u64, var: f32) {
        let mut p = Vec::new();
        for v in [var, var, 0.0, 0.1, 0.0] {
            p.extend_from_slice(&f32::to_le_bytes(v));
        }
        p.extend_from_slice(&[0, 0]);
        self.frame(t_ms, 193, &p);
    }
}

/// The time rules (EKF variances for 1 s, battery below BATT_LOW_VOLT for
/// more than BATT_LOW_TIMER, heartbeat gaps longer than FS_GCS_TIMEOUT) on a
/// tlog in real time, where the SITL autotest tlogs run at SIM_SPEEDUP.
#[test]
fn tlog_time_rules_in_real_time() {
    let mut t = Tl {
        bytes: Vec::new(),
        seq: 0,
    };
    for ms in (0..=60_000u64).step_by(100) {
        let s = ms / 1000;
        // heartbeats once a second, none from 20 s to 30 s
        if ms % 1000 == 0 && !(21..30).contains(&s) {
            t.heartbeat(ms, ms < 60_000);
        }
        if ms % 1000 == 0 {
            // 10.0 V from 35 s to 50 s (15 s), 10.2 V from 52 s to 55 s (3 s)
            let mv = if (35..=50).contains(&s) {
                10_000
            } else if (52..=55).contains(&s) {
                10_200
            } else {
                12_000
            };
            t.sys_status(ms, mv);
        }
        // velocity and position variances 1.0 from 40.0 s to 41.5 s, and
        // for 0.5 s from 45.0 s
        let bad = (40_000..=41_500).contains(&ms) || (45_000..=45_500).contains(&ms);
        t.ekf(ms, if bad { 1.0 } else { 0.1 });
    }
    let log = Log::read(&t.bytes, &ReadOptions::default()).unwrap();
    assert_eq!(log.kind, LogKind::Tlog);
    let doc = facts::run(&log, &facts::Options::default());
    assert_eq!(doc.log.duration_s, 60.0);

    let gcs = check(&doc, "link.gcs_failsafe");
    assert_eq!(gcs.status, Status::Warn, "{}", gcs.summary);
    assert_eq!(gcs.ranges.len(), 1);
    assert_eq!((gcs.ranges[0].start_s, gcs.ranges[0].end_s), (20.0, 30.0));

    let bat = check(&doc, "battery.voltage");
    let long: Vec<_> = bat
        .ranges
        .iter()
        .filter(|r| r.detail.contains("for more than"))
        .collect();
    assert_eq!(long.len(), 1, "{:?}", bat.ranges);
    assert_eq!((long[0].start_s, long[0].end_s), (35.0, 50.0));
    assert_eq!(bat.ranges.len(), 2, "{:?}", bat.ranges);

    let ekf = check(&doc, "ekf.variance");
    assert_eq!(ekf.status, Status::Fail, "{}", ekf.summary);
    let two: Vec<_> = ekf
        .ranges
        .iter()
        .filter(|r| r.detail.contains("two or more"))
        .collect();
    assert_eq!(two.len(), 1, "{:?}", ekf.ranges);
    assert_eq!((two[0].start_s, two[0].end_s), (40.0, 41.5));
}

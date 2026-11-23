//! flight-report on the ArduPilot SITL autotest logs and the PX4 sample
//! logs in `testdata/external` (fetched by tools/fetch_testdata.sh; the test
//! is skipped when a log is absent). Every asserted number is read from the
//! real log, not invented.

#![cfg(feature = "facts")]

use std::path::{Path, PathBuf};

use flightlog_core::report::{self, EndCause, ReportOptions, PART_107_CEILING_M};
use flightlog_core::{FlightLog, ReadOptions};

fn external(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../testdata/external")
        .join(name);
    if p.exists() {
        Some(p)
    } else {
        println!("SKIP {name}: not downloaded (tools/fetch_testdata.sh)");
        None
    }
}

fn build(name: &str, opts: &ReportOptions) -> Option<report::FlightReport> {
    let p = external(name)?;
    let log = FlightLog::open(&p, &ReadOptions::default()).unwrap();
    Some(report::build(&log, opts))
}

fn build_ulog(name: &str, opts: &ReportOptions) -> Option<report::FlightReport> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../testdata/external/ulog")
        .join(name);
    if !p.exists() {
        println!("SKIP {name}: not downloaded (tools/fetch_testdata.sh)");
        return None;
    }
    let log = FlightLog::open(&p, &ReadOptions::default()).unwrap();
    Some(report::build(&log, opts))
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// A clean autopilot mission: real flight numbers, a normal landing, and
/// (because the mission tops out near 40 m) well under the 400 ft ceiling.
#[test]
fn mission_bin_summary_and_normal_landing() {
    let Some(r) = build("copter-mission.bin", &ReportOptions::default()) else {
        return;
    };
    let s = &r.summary;
    assert_eq!(s.format, "dataflash");
    assert!(s.flew);
    // Armed about 196 s (ARM.ArmState 41.1 s to disarm 236.8 s).
    assert!(
        approx(s.armed_s.unwrap(), 195.73, 1.0),
        "armed_s {:?}",
        s.armed_s
    );
    // Peak height above home 39.97 m from POS.RelHomeAlt.
    assert_eq!(s.alt_source.as_deref(), Some("POS.RelHomeAlt"));
    assert!(
        approx(s.max_alt_agl_m.unwrap(), 39.97, 0.5),
        "alt {:?}",
        s.max_alt_agl_m
    );
    // Flew out to ~305 m from takeoff, top GPS ground speed ~10 m/s.
    assert_eq!(s.distance_source.as_deref(), Some("GPS"));
    assert!(
        approx(s.max_distance_m.unwrap(), 304.6, 5.0),
        "dist {:?}",
        s.max_distance_m
    );
    assert_eq!(s.speed_source.as_deref(), Some("GPS.Spd"));
    assert!(
        approx(s.top_speed_ms.unwrap(), 10.04, 0.5),
        "spd {:?}",
        s.top_speed_ms
    );
    assert!(s.takeoff.is_some() && s.landing.is_some());
    assert!(s.gps_fixes > 1000);

    assert_eq!(r.ending.cause, EndCause::NormalLanding);
    assert!(!r.ending.ended_armed);
    assert_eq!(r.ending.failsafe, None);

    // 400 ft ceiling: not exceeded.
    assert!(!r.altitude.exceeded);
    assert!(approx(r.altitude.ceiling_agl_m, PART_107_CEILING_M, 1e-6));
}

/// Same mission, a 30 m ceiling: the real 40 m peak now reads as a violation
/// with the time range it happened in.
#[test]
fn mission_bin_ceiling_violation_at_30m() {
    let opts = ReportOptions {
        ceiling_agl_m: 30.0,
        ..Default::default()
    };
    let Some(r) = build("copter-mission.bin", &opts) else {
        return;
    };
    assert!(r.altitude.exceeded, "{}", r.altitude.summary);
    assert!(!r.altitude.ranges.is_empty());
    let peak = r
        .altitude
        .ranges
        .iter()
        .map(|c| c.peak_m)
        .fold(0.0, f64::max);
    assert!(approx(peak, 39.97, 0.5), "peak {peak}");
    for c in &r.altitude.ranges {
        assert!(c.end_s >= c.start_s);
        assert!(c.peak_m > 30.0);
    }
}

/// ArduPlane/Copter radio failsafe: the flight ended because the radio
/// failsafe forced RTL. The failsafe family and the mode are read from the log.
#[test]
fn throttle_failsafe_bin_ends_on_radio_failsafe() {
    let Some(r) = build("copter-throttle-failsafe.bin", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.ending.cause, EndCause::FailsafeLanding);
    assert_eq!(r.ending.failsafe.as_deref(), Some("Radio Failsafe"));
    assert_eq!(r.ending.final_mode.as_deref(), Some("RTL"));
    assert!(r.ending.summary.contains("RTL"), "{}", r.ending.summary);
    assert!(!r.ending.evidence.is_empty());
}

/// Battery failsafe: the flight ended because the battery failsafe forced LAND.
#[test]
fn battery_failsafe_bin_ends_on_battery_failsafe() {
    let Some(r) = build("copter-battery-failsafe.bin", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.ending.cause, EndCause::FailsafeLanding);
    assert_eq!(r.ending.failsafe.as_deref(), Some("Battery Failsafe"));
    assert_eq!(r.ending.final_mode.as_deref(), Some("LAND"));
}

/// A GPS glitch that recovered mid-flight: the vehicle still landed normally,
/// so the end cause is a normal landing, not a failsafe.
#[test]
fn gps_glitch_bin_recovers_and_lands() {
    let Some(r) = build("copter-gps-glitch.bin", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.ending.cause, EndCause::NormalLanding);
    assert_eq!(r.ending.failsafe, None);
    assert!(r.summary.flew);
}

/// The ground-station tlog of the same flight gives the same height, distance
/// and speed (within rounding) as the onboard DataFlash log, read from the
/// tlog's own message types.
#[test]
fn mission_tlog_matches_bin_metrics() {
    let (Some(b), Some(t)) = (
        build("copter-mission.bin", &ReportOptions::default()),
        build("copter-mission.tlog", &ReportOptions::default()),
    ) else {
        return;
    };
    assert_eq!(t.summary.format, "tlog");
    assert_eq!(
        t.summary.alt_source.as_deref(),
        Some("GLOBAL_POSITION_INT.relative_alt")
    );
    assert_eq!(
        t.summary.speed_source.as_deref(),
        Some("VFR_HUD.groundspeed")
    );
    assert!(
        approx(
            b.summary.max_alt_agl_m.unwrap(),
            t.summary.max_alt_agl_m.unwrap(),
            0.5
        ),
        "alt bin {:?} tlog {:?}",
        b.summary.max_alt_agl_m,
        t.summary.max_alt_agl_m
    );
    assert!(
        approx(
            b.summary.max_distance_m.unwrap(),
            t.summary.max_distance_m.unwrap(),
            2.0
        ),
        "dist bin {:?} tlog {:?}",
        b.summary.max_distance_m,
        t.summary.max_distance_m
    );
    assert!(
        approx(
            b.summary.top_speed_ms.unwrap(),
            t.summary.top_speed_ms.unwrap(),
            0.5
        ),
        "spd bin {:?} tlog {:?}",
        b.summary.top_speed_ms,
        t.summary.top_speed_ms
    );
}

/// A PX4 bench log that never armed: the report says so honestly, reads the
/// real (near-zero) height from vehicle_local_position, and still serializes.
#[test]
fn px4_bench_log_reports_no_flight() {
    let Some(r) = build("sample.ulg", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.summary.format, "ulog");
    assert!(!r.summary.flew, "bench log should not report a flight");
    assert_eq!(r.ending.cause, EndCause::Undetermined);
    // Height read from the right PX4 field; the vehicle stayed on the ground.
    assert_eq!(
        r.summary.alt_source.as_deref(),
        Some("vehicle_local_position.z")
    );
    assert!(r.summary.max_alt_agl_m.unwrap().abs() < 1.0);
    // The report round-trips through JSON.
    let json = r.to_json();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["summary"]["flew"], false);
    assert!(v["altitude"]["citation"]
        .as_str()
        .unwrap()
        .contains("107.51"));
}

/// A PX4 log with real (small) motion: height, distance and speed come from
/// vehicle_local_position, not from a GPS track.
#[test]
fn px4_local_position_metrics() {
    let Some(r) = build("sample_appended.ulg", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.summary.format, "ulog");
    // Real measured peak height above origin is about 0.885 m.
    assert!(
        approx(r.summary.max_alt_agl_m.unwrap(), 0.885, 0.05),
        "{:?}",
        r.summary.max_alt_agl_m
    );
    assert_eq!(
        r.summary.distance_source.as_deref(),
        Some("vehicle_local_position.x,y")
    );
    assert_eq!(r.summary.gps_fixes, 0);
}

/// A real PX4 flight (pyulog's `sample_px4_events.ulg`): the vehicle took
/// off, flew, and landed and disarmed. The report reads that as an armed
/// flight with a normal landing, height from vehicle_local_position.
#[test]
fn px4_events_flight_is_a_normal_landing() {
    let Some(r) = build_ulog("sample_px4_events.ulg", &ReportOptions::default()) else {
        return;
    };
    assert_eq!(r.summary.format, "ulog");
    assert!(r.summary.flew, "an armed PX4 flight");
    assert!(r.summary.armed_s.unwrap() > 5.0);
    assert_eq!(
        r.summary.alt_source.as_deref(),
        Some("vehicle_local_position.z")
    );
    assert!(r.summary.max_alt_agl_m.unwrap() > 0.5);
    assert_eq!(r.ending.cause, EndCause::NormalLanding);
    assert!(!r.ending.ended_armed);
}

/// The same log always gives the same report.
#[test]
fn deterministic() {
    let Some(p) = external("copter-mission.bin") else {
        return;
    };
    let log = FlightLog::open(&p, &ReadOptions::default()).unwrap();
    let a = report::build(&log, &ReportOptions::default());
    let b = report::build(&log, &ReportOptions::default());
    assert_eq!(a.to_json(), b.to_json());
}

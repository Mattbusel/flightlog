//! flight-facts on the ArduPilot SITL autotest logs (testdata/external,
//! fetched by tools/fetch_testdata.sh; skipped when absent) and on the
//! committed fixtures.

#![cfg(feature = "facts")]

use std::path::{Path, PathBuf};

use flightlog_core::facts::{self, Status};
use flightlog_core::{Log, ReadOptions};

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

fn run(path: &Path) -> facts::FlightFacts {
    let log = Log::open(path, &ReadOptions::default()).unwrap();
    facts::run(&log, &facts::Options::default())
}

fn status(doc: &facts::FlightFacts, id: &str) -> Status {
    doc.check(id)
        .unwrap_or_else(|| panic!("no check {id}"))
        .status
}

const ALL: [&str; 12] = [
    "vibration.levels",
    "vibration.clipping",
    "vibration.failsafe",
    "ekf.variance",
    "gps.hdop",
    "gps.satellites",
    "gps.glitch",
    "battery.voltage",
    "power.board_vcc",
    "power.log_ends_armed",
    "rc.failsafe",
    "link.gcs_failsafe",
];

#[test]
fn fixtures_have_every_check() {
    for f in ["mini.bin", "mini.tlog"] {
        let doc = run(&Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(f));
        let ids: Vec<&str> = doc.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ALL);
        for c in &doc.checks {
            assert!(!c.sources.is_empty(), "{} has no wiki source", c.id);
            for s in &c.sources {
                assert!(s.url.starts_with("https://ardupilot.org/"));
                assert_eq!(s.wiki_commit.len(), 40);
            }
        }
        let v: serde_json::Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(v["flight_facts_version"], "1.0");
    }
}

#[test]
fn same_log_same_document() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mini.tlog");
    assert_eq!(run(&p).to_json(), run(&p).to_json());
}

/// Each autotest injects one fault; the check for that fault must report it
/// with at least one time range.
#[test]
fn autotest_faults_are_found() {
    let cases: &[(&str, &str, Status)] = &[
        ("copter-throttle-failsafe.bin", "rc.failsafe", Status::Fail),
        ("copter-throttle-failsafe.bin", "ekf.variance", Status::Fail),
        ("copter-gcs-failsafe.bin", "link.gcs_failsafe", Status::Fail),
        (
            "copter-gcs-failsafe.tlog",
            "link.gcs_failsafe",
            Status::Fail,
        ),
        (
            "copter-battery-failsafe.bin",
            "battery.voltage",
            Status::Fail,
        ),
        (
            "copter-vibration-failsafe.bin",
            "vibration.failsafe",
            Status::Warn,
        ),
        (
            "copter-vibration-failsafe.tlog",
            "vibration.failsafe",
            Status::Warn,
        ),
        ("copter-gps-glitch.bin", "gps.glitch", Status::Fail),
        ("copter-gps-glitch.tlog", "gps.glitch", Status::Fail),
    ];
    for (log, id, want) in cases {
        let Some(p) = external(log) else { continue };
        let doc = run(&p);
        let c = doc.check(id).unwrap();
        assert_eq!(c.status, *want, "{log} {id}: {}", c.summary);
        assert!(!c.ranges.is_empty(), "{log} {id} has no time range");
    }
}

#[test]
fn clean_mission_passes() {
    for log in ["copter-mission.bin", "copter-mission.tlog"] {
        let Some(p) = external(log) else { continue };
        let doc = run(&p);
        for id in ALL {
            let s = status(&doc, id);
            // SITL simulates 10 satellites, below the wiki's 12
            let ok = matches!(s, Status::Pass | Status::NotApplicable)
                || (id == "gps.satellites" && s == Status::Warn);
            assert!(ok, "{log} {id}: {:?} {}", s, doc.check(id).unwrap().summary);
        }
        assert_eq!(doc.log.vehicle.as_deref(), Some("copter"));
        assert!(doc
            .log
            .firmware
            .as_deref()
            .unwrap()
            .starts_with("ArduCopter V4.8.0-dev"));
    }
}

#[test]
fn damaged_logs_still_give_facts() {
    for log in [
        "copter-mission-truncated.bin",
        "copter-mission-bitflip.bin",
        "copter-mission-zeroblock.bin",
        "copter-mission-garbage.bin",
    ] {
        let Some(p) = external(log) else { continue };
        let doc = run(&p);
        assert!(
            !doc.integrity.clean || doc.integrity.truncated_tail_bytes > 0,
            "{log}"
        );
        assert_eq!(status(&doc, "vibration.levels"), Status::Pass, "{log}");
    }
}

#[test]
fn bitflip_log_recovers() {
    // pymavlink 2.4.50's DFReader_binary raises on this file (a flipped byte
    // in the XKT format string); flightlog drops only that message type.
    let (Some(p), Some(clean)) = (
        external("copter-mission-bitflip.bin"),
        external("copter-mission.bin"),
    ) else {
        return;
    };
    let a = Log::open(&p, &ReadOptions::default()).unwrap();
    let b = Log::open(&clean, &ReadOptions::default()).unwrap();
    assert_eq!(a.integrity.invalid_formats.len(), 1);
    assert!(a.table("XKT").is_none());
    let lost = b.integrity.messages - a.integrity.messages;
    assert!(lost < 200, "lost {lost} messages");
}

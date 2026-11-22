//! GPS track, events and FlightLog on the committed ArduPilot fixtures.

use flightlog_core::{track, FlightLog, Log, LogKind, ReadOptions};

fn open(name: &str, opts: &ReadOptions) -> Log {
    let p = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    Log::open(p, opts).unwrap()
}

/// A downloaded SITL log (tools/fetch_testdata.sh), `None` when missing.
fn sitl(name: &str, opts: &ReadOptions) -> Option<Log> {
    let p = format!("{}/../testdata/external/{name}", env!("CARGO_MANIFEST_DIR"));
    match Log::open(&p, opts) {
        Ok(l) => Some(l),
        Err(_) => {
            println!("SKIP {p}");
            None
        }
    }
}

fn check_points(pts: &[track::TrackPoint]) {
    assert!(!pts.is_empty());
    for p in pts {
        // ArduPilot SITL home: CMAC, Canberra
        assert!((p.lat + 35.36).abs() < 0.1, "{p:?}");
        assert!((p.lon - 149.16).abs() < 0.1, "{p:?}");
        let alt = p.alt_m.unwrap();
        assert!((400.0..800.0).contains(&alt), "{p:?}");
    }
    for w in pts.windows(2) {
        assert!(w[0].time_us <= w[1].time_us);
    }
}

#[test]
fn dataflash_track_scaled_and_raw_agree() {
    let Some(log) = sitl("copter-mission.bin", &ReadOptions::default()) else {
        return;
    };
    let scaled = track::from_log(&log);
    let raw_opts = ReadOptions {
        scale: false,
        ..Default::default()
    };
    let raw = track::from_log(&sitl("copter-mission.bin", &raw_opts).unwrap());
    check_points(&scaled);
    assert_eq!(scaled.len(), raw.len());
    for (a, b) in scaled.iter().zip(&raw) {
        assert!((a.lat - b.lat).abs() < 1e-9 && (a.lon - b.lon).abs() < 1e-9);
        assert!((a.alt_m.unwrap() - b.alt_m.unwrap()).abs() < 1e-6);
        assert_eq!(a.utc_us, b.utc_us);
    }
    // GPS time is known in SITL
    assert!(scaled[0].utc_us.unwrap() > 1_600_000_000_000_000);
    let kml = track::to_kml(&scaled, "copter-mission.bin");
    assert!(kml.contains("<altitudeMode>absolute</altitudeMode>"));
}

#[test]
fn tlog_track() {
    let log = open("mini.tlog", &ReadOptions::default());
    assert_eq!(log.kind, LogKind::Tlog);
    let pts = track::from_log(&log);
    check_points(&pts);
    assert_eq!(pts[0].utc_us, Some(pts[0].time_us as i64));
}

#[test]
fn flight_log_wraps_ardupilot() {
    let p = format!("{}/tests/data/mini.bin", env!("CARGO_MANIFEST_DIR"));
    let f = FlightLog::open(&p, &ReadOptions::default()).unwrap();
    assert_eq!(f.format_name(), "dataflash");
    assert_eq!(f.time_column(), "TimeUS");
    let log = f.as_ardupilot().unwrap();
    assert_eq!(f.params().len(), log.params.len());
    assert_eq!(f.time_range_us(), log.time_range_us());
    assert_eq!(f.track(), track::from_log(log));
    assert!(f.as_px4().is_none());
    #[cfg(feature = "facts")]
    {
        let ev = f.events();
        for w in ev.windows(2) {
            assert!(w[0].time_us <= w[1].time_us);
        }
    }
}

//! ULog files built byte by byte: every message type, nested formats,
//! multi-instance topics, damage, appended data, events and GPS track.

use flightlog_core::events::{self, EventKind};
use flightlog_core::ulog::{InfoValue, ULog, SYNC};
use flightlog_core::{track, ColumnData, FlightLog, ReadOptions};

#[derive(Default)]
struct W {
    b: Vec<u8>,
}

impl W {
    fn new() -> W {
        let mut w = W::default();
        w.b.extend_from_slice(b"ULog\x01\x12\x35\x01");
        w.b.extend_from_slice(&1000u64.to_le_bytes());
        w
    }
    fn msg(&mut self, ty: u8, p: &[u8]) -> &mut W {
        self.b.extend_from_slice(&(p.len() as u16).to_le_bytes());
        self.b.push(ty);
        self.b.extend_from_slice(p);
        self
    }
    fn flags(&mut self, incompat: u8, offsets: [u64; 3]) -> &mut W {
        let mut p = vec![0u8; 8];
        p.push(incompat);
        p.extend_from_slice(&[0; 7]);
        for o in offsets {
            p.extend_from_slice(&o.to_le_bytes());
        }
        self.msg(b'B', &p)
    }
    fn format(&mut self, s: &str) -> &mut W {
        self.msg(b'F', s.as_bytes())
    }
    fn keyed(prefix: &[u8], key: &str, v: &[u8]) -> Vec<u8> {
        let mut p = prefix.to_vec();
        p.push(key.len() as u8);
        p.extend_from_slice(key.as_bytes());
        p.extend_from_slice(v);
        p
    }
    fn info(&mut self, key: &str, v: &[u8]) -> &mut W {
        let p = W::keyed(&[], key, v);
        self.msg(b'I', &p)
    }
    fn param_f(&mut self, name: &str, v: f32) -> &mut W {
        let p = W::keyed(&[], &format!("float {name}"), &v.to_le_bytes());
        self.msg(b'P', &p)
    }
    fn param_i(&mut self, name: &str, v: i32) -> &mut W {
        let p = W::keyed(&[], &format!("int32_t {name}"), &v.to_le_bytes());
        self.msg(b'P', &p)
    }
    fn add(&mut self, multi: u8, id: u16, name: &str) -> &mut W {
        let mut p = vec![multi];
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(name.as_bytes());
        self.msg(b'A', &p)
    }
    fn data(&mut self, id: u16, d: &[u8]) -> &mut W {
        let mut p = id.to_le_bytes().to_vec();
        p.extend_from_slice(d);
        self.msg(b'D', &p)
    }
    fn log(&mut self, level: u8, t: u64, s: &str) -> &mut W {
        let mut p = vec![b'0' + level];
        p.extend_from_slice(&t.to_le_bytes());
        p.extend_from_slice(s.as_bytes());
        self.msg(b'L', &p)
    }
}

/// `point`: nested type with an array; `topic` uses it plus trailing padding.
fn base() -> W {
    let mut w = W::new();
    w.flags(0, [0; 3])
        .info("char[5] sys_name", b"PX4\0\0")
        .info("uint32_t ver_sw_release", &0x010e_0000u32.to_le_bytes())
        .format("point:float x;int16_t[2] v;")
        .format("topic:uint64_t timestamp;point p;bool ok;uint8_t[3] _padding0;")
        .param_f("MPC_XY_VEL_MAX", 12.0)
        .param_i("COM_RC_LOSS_T", 5)
        .msg(
            b'Q',
            &W::keyed(&[1], "float MPC_XY_VEL_MAX", &10.0f32.to_le_bytes()),
        )
        .msg(
            b'Q',
            &W::keyed(&[3], "int32_t COM_RC_LOSS_T", &5i32.to_le_bytes()),
        )
        .add(0, 7, "topic")
        .add(1, 8, "topic");
    w
}

fn topic(t: u64, x: f32, v: [i16; 2], ok: bool, padding: bool) -> Vec<u8> {
    let mut d = t.to_le_bytes().to_vec();
    d.extend_from_slice(&x.to_le_bytes());
    d.extend_from_slice(&v[0].to_le_bytes());
    d.extend_from_slice(&v[1].to_le_bytes());
    d.push(ok as u8);
    if padding {
        d.extend_from_slice(&[0; 3]);
    }
    d
}

#[test]
fn every_message_type() {
    let mut w = base();
    w.data(7, &topic(2000, 1.5, [1, -2], true, true))
        .data(8, &topic(2001, 2.5, [3, 4], false, false)) // trailing padding left out
        .log(4, 2500, "low battery")
        .msg(b'O', &30u16.to_le_bytes())
        .param_f("MPC_XY_VEL_MAX", 8.0)
        .msg(b'S', &SYNC)
        .data(7, &topic(3000, -1.0, [5, 6], true, true));
    let mut c = vec![b'3'];
    c.extend_from_slice(&9u16.to_le_bytes());
    c.extend_from_slice(&3500u64.to_le_bytes());
    c.extend_from_slice(b"tagged");
    w.msg(b'C', &c);
    let log = ULog::read(&w.b).unwrap();

    assert_eq!(log.version, 1);
    assert_eq!(log.start_timestamp_us, 1000);
    assert_eq!(log.info_str("sys_name"), Some("PX4"));
    assert_eq!(log.info["ver_sw_release"], InfoValue::UInt(0x010e_0000));
    assert_eq!(log.initial_params["MPC_XY_VEL_MAX"], 12.0);
    assert_eq!(log.param("MPC_XY_VEL_MAX"), Some(8.0));
    assert_eq!(log.param("COM_RC_LOSS_T"), Some(5.0));
    assert_eq!(log.changed_params.len(), 1);
    assert_eq!(log.changed_params[0].timestamp_us, 2001);
    let d = log.default_params["COM_RC_LOSS_T"];
    assert_eq!((d.system, d.airframe), (Some(5.0), Some(5.0)));
    assert_eq!(
        log.non_default_params(),
        vec![("MPC_XY_VEL_MAX", 8.0, 10.0)]
    );

    let t = log.table("topic").unwrap();
    assert_eq!(
        t.column_names(),
        vec!["timestamp", "p.x", "p.v[0]", "p.v[1]", "ok", "multi_id"]
    );
    assert_eq!(t.len(), 3);
    assert_eq!(t.f64s("p.x").unwrap(), vec![1.5, 2.5, -1.0]);
    assert_eq!(t.f64s("p.v[1]").unwrap(), vec![-2.0, 4.0, 6.0]);
    assert_eq!(
        t.column("ok").unwrap().data,
        ColumnData::Int8(vec![1, 0, 1])
    );
    assert_eq!(t.instances(), vec![0, 1]);
    assert_eq!(t.instance(1).len(), 1);
    assert_eq!(log.counts["topic"], 3);

    assert_eq!(log.logged.len(), 2);
    assert_eq!(log.logged[0].level_name(), "WARNING");
    assert_eq!(log.logged[1].tag, Some(9));
    assert_eq!(log.logged[1].text, "tagged");
    assert_eq!(log.dropouts[0].duration_ms, 30);
    assert_eq!(log.dropouts[0].timestamp_us, 2001);
    assert_eq!(log.sync_count, 1);
    assert!(log.integrity.is_clean(), "{:?}", log.integrity);
    assert_eq!(log.integrity.truncated_tail_bytes, 0);
    assert_eq!(log.time_range_us(), Some((2000, 3000)));
}

#[test]
fn not_ulog() {
    assert!(ULog::read(b"ULog").is_err());
    assert!(ULog::read(b"\xa3\x95\x80 something else").is_err());
    assert!(FlightLog::read(b"nothing", &ReadOptions::default()).is_err());
}

#[test]
fn damage_resyncs_on_the_sync_sequence() {
    let mut w = base();
    w.data(7, &topic(2000, 1.0, [0, 0], true, true));
    let garbage_at = w.b.len();
    w.b.extend_from_slice(&[0u8; 100]);
    w.msg(b'S', &SYNC)
        .data(7, &topic(2100, 2.0, [0, 0], true, true))
        // a data message too short for its topic
        .data(7, &[1, 2, 3])
        // a message id that was never subscribed
        .data(99, &[0; 4]);
    let log = ULog::read(&w.b).unwrap();
    assert_eq!(log.table("topic").unwrap().len(), 2);
    assert!(!log.integrity.is_clean());
    // the garbage, the sync message (its header was garbage too), and the two bad data messages
    assert_eq!(log.integrity.skipped_regions[0].offset, garbage_at as u64);
    assert!(log.integrity.skipped_bytes >= 100);
    assert_eq!(log.integrity.unknown_types.get(&99), Some(&1));
}

#[test]
fn truncated_file() {
    let mut w = base();
    w.data(7, &topic(2000, 1.0, [0, 0], true, true));
    w.data(7, &topic(2100, 2.0, [0, 0], true, true));
    let n = w.b.len();
    let log = ULog::read(&w.b[..n - 5]).unwrap();
    assert_eq!(log.table("topic").unwrap().len(), 1);
    assert_eq!(log.integrity.truncated_tail_bytes, 3 + 2 + 20 - 5);
    assert!(log.integrity.is_clean());
}

#[test]
fn appended_data_after_a_crash() {
    // the logger reserved an offset for data appended after the crash
    let mut w = W::new();
    w.flags(1, [0; 3]);
    let flags_offsets_at = w.b.len() - 24;
    w.format("topic:uint64_t timestamp;point p;bool ok;uint8_t[3] _padding0;")
        .format("point:float x;int16_t[2] v;")
        .add(0, 7, "topic")
        .data(7, &topic(2000, 1.0, [0, 0], true, true));
    // a message cut off by the crash
    let cut = w.b.len();
    w.data(7, &topic(2100, 2.0, [0, 0], true, true));
    w.b.truncate(cut + 10);
    let offset = w.b.len() as u64;
    w.data(7, &topic(5000, 3.0, [0, 0], true, true));
    w.b[flags_offsets_at..flags_offsets_at + 8].copy_from_slice(&offset.to_le_bytes());
    let log = ULog::read(&w.b).unwrap();
    assert_eq!(log.appended_offsets, vec![offset]);
    assert_eq!(
        log.table("topic").unwrap().f64s("p.x").unwrap(),
        vec![1.0, 3.0]
    );
    assert_eq!(log.integrity.skipped_bytes, 10);
}

#[test]
fn hostile_formats_do_not_hang_or_panic() {
    let mut w = W::new();
    w.format("a:a self;")
        .format("big:double[65535] v;")
        .format("huge:big[65535] b;")
        .format("broken")
        .add(0, 1, "a")
        .add(0, 2, "huge")
        .add(0, 3, "missing")
        .data(1, &[0; 8]);
    let log = ULog::read(&w.b).unwrap();
    assert!(log.tables().is_empty());
    assert_eq!(log.integrity.invalid_formats.len(), 4);
}

#[test]
fn arbitrary_bytes_after_the_header() {
    // cheap deterministic fuzz: no panic, no hang
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for round in 0..200 {
        let mut b = base().b;
        for _ in 0..(round * 7 % 900) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b.push(x as u8);
        }
        let _ = ULog::read(&b);
        b.truncate(16 + round % (b.len() - 16));
        let _ = ULog::read(&b);
    }
}

fn status(t: u64, nav: u8, arming: u8, failsafe: bool, fd: u16) -> Vec<u8> {
    let mut d = t.to_le_bytes().to_vec();
    d.extend_from_slice(&[nav, arming, failsafe as u8]);
    d.extend_from_slice(&fd.to_le_bytes());
    d
}

fn gps(t: u64, utc: u64, lat: i32, lon: i32, alt_mm: i32, fix: u8) -> Vec<u8> {
    let mut d = t.to_le_bytes().to_vec();
    d.extend_from_slice(&utc.to_le_bytes());
    d.extend_from_slice(&lat.to_le_bytes());
    d.extend_from_slice(&lon.to_le_bytes());
    d.extend_from_slice(&alt_mm.to_le_bytes());
    d.push(fix);
    d
}

fn flight() -> Vec<u8> {
    let mut w = W::new();
    w.format("vehicle_status:uint64_t timestamp;uint8_t nav_state;uint8_t arming_state;bool failsafe;uint16_t failure_detector_status;")
        .format("vehicle_gps_position:uint64_t timestamp;uint64_t time_utc_usec;int32_t lat;int32_t lon;int32_t alt;uint8_t fix_type;")
        .add(0, 1, "vehicle_status")
        .add(0, 2, "vehicle_gps_position")
        .data(1, &status(1_000_000, 4, 1, false, 0))
        .data(2, &gps(1_100_000, 0, 0, 0, 0, 0))
        .data(2, &gps(1_200_000, 1_700_000_000_000_000, 473_977_418, 85_455_939, 488_000, 3))
        .data(1, &status(2_000_000, 17, 2, false, 0))
        .data(1, &status(3_000_000, 17, 2, false, 0))
        .log(6, 3_100_000, "info only")
        .log(3, 3_200_000, "Motor failure")
        .data(1, &status(4_000_000, 5, 2, true, 3))
        .data(2, &gps(4_100_000, 1_700_000_002_900_000, 473_977_500, 85_456_000, 470_000, 4))
        .data(1, &status(5_000_000, 5, 1, false, 3));
    w.b
}

#[test]
fn px4_events() {
    let log = ULog::read(&flight()).unwrap();
    let ev = events::from_ulog(&log);
    let got: Vec<(u64, &str, &str)> = ev
        .iter()
        .map(|e| (e.time_us, e.kind.name(), e.text.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (1_000_000, "mode", "Hold"),
            (2_000_000, "mode", "Takeoff"),
            (2_000_000, "armed", "armed"),
            (3_200_000, "message", "[ERROR] Motor failure"),
            (4_000_000, "mode", "Return"),
            (4_000_000, "failsafe", "failsafe"),
            (
                4_000_000,
                "failure_detector",
                "failure detector: roll, pitch"
            ),
            (5_000_000, "disarmed", "disarmed"),
            (5_000_000, "failsafe_cleared", "failsafe cleared"),
        ]
    );
    assert_eq!(ev[6].code, Some(3));
    assert_eq!(ev[6].kind, EventKind::FailureDetector);
}

#[test]
fn px4_track() {
    let log = ULog::read(&flight()).unwrap();
    let pts = track::from_ulog(&log);
    assert_eq!(pts.len(), 2, "no-fix sample dropped");
    assert_eq!(pts[0].lat, 47.397_741_8);
    assert_eq!(pts[0].alt_m, Some(488.0));
    assert_eq!(pts[1].utc_us, Some(1_700_000_002_900_000));
    assert_eq!(log.utc_offset_us(), Some(1_700_000_000_000_000 - 1_200_000));
    let gpx = track::to_gpx(&pts, "f");
    assert!(gpx.contains("<trkpt lat=\"47.3977418\" lon=\"8.5455939\"><ele>488.00</ele><time>2023-11-14T22:13:20.000Z</time></trkpt>"));
    let f = FlightLog::read(&flight(), &ReadOptions::default()).unwrap();
    assert_eq!(f.track(), pts);
    assert!(f.as_px4().is_some());
}

#[test]
fn px4_track_new_field_names() {
    // PX4 v1.15+: sensor_gps with degrees and metres as doubles
    let mut w = W::new();
    w.format("sensor_gps:uint64_t timestamp;double latitude_deg;double longitude_deg;double altitude_msl_m;uint8_t fix_type;")
        .add(0, 1, "sensor_gps");
    let mut d = 7u64.to_le_bytes().to_vec();
    for v in [47.5f64, 8.5, 400.25] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.push(3);
    w.data(1, &d);
    let pts = track::from_ulog(&ULog::read(&w.b).unwrap());
    assert_eq!(pts.len(), 1);
    assert_eq!(
        (pts[0].lat, pts[0].lon, pts[0].alt_m),
        (47.5, 8.5, Some(400.25))
    );
    assert_eq!(pts[0].utc_us, None);
}

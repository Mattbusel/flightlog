//! GPS track of a flight, and GPX / KML export.
//!
//! [`from_log`] reads the track from an ArduPilot DataFlash log (`GPS`,
//! first receiver) or tlog (`GPS_RAW_INT`), [`from_ulog`] from a PX4 ULog
//! (`vehicle_gps_position`, else the first `sensor_gps`). Only samples with
//! at least a 3D fix are kept, and positions of exactly 0, 0 (what a
//! receiver reports before its first fix) are dropped.
//!
//! ```no_run
//! use flightlog_core::{track, Log, ReadOptions};
//!
//! let log = Log::open("00000001.BIN", &ReadOptions::default())?;
//! let points = track::from_log(&log);
//! std::fs::write("flight.gpx", track::to_gpx(&points, "00000001.BIN"))?;
//! std::fs::write("flight.kml", track::to_kml(&points, "00000001.BIN"))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::column::{ColumnData, Table};
use crate::log::{Log, LogKind};
use crate::ulog::ULog;

/// One GPS sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    /// Timestamp in the log's time base (DataFlash and ULog: microseconds
    /// since boot; tlog: Unix microseconds).
    pub time_us: u64,
    /// Unix time in microseconds, when the log has GPS time.
    pub utc_us: Option<i64>,
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// Altitude above mean sea level in metres.
    pub alt_m: Option<f64>,
}

/// GPS fix types below this (0 no GPS, 1 no fix, 2 2D) are left out.
const MIN_FIX: i64 = 3;

fn valid(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// Degrees from a column that is either scaled (float) or raw `1e7` integers.
fn degrees(t: &Table, name: &str, i: usize) -> Option<f64> {
    let c = t.column(name)?;
    let v = c.data.get_f64(i)?;
    Some(match c.data {
        ColumnData::Float32(_) | ColumnData::Float64(_) => v,
        _ => v / 1e7,
    })
}

/// The first instance of a table with an instance field.
fn primary(t: &Table) -> Table {
    match t.instances().first() {
        Some(&k) if t.instance_field.is_some() => t.instance(k),
        _ => t.clone(),
    }
}

/// The GPS track of an ArduPilot DataFlash log or tlog.
pub fn from_log(log: &Log) -> Vec<TrackPoint> {
    let mut out = Vec::new();
    match log.kind {
        LogKind::DataFlash => {
            let Some(t) = log.table("GPS").map(primary) else {
                return out;
            };
            let alt = t.column("Alt");
            for i in 0..t.len() {
                let (Some(time), Some(lat), Some(lon)) = (
                    t.column("TimeUS").and_then(|c| c.data.get_i64(i)),
                    degrees(&t, "Lat", i),
                    degrees(&t, "Lng", i),
                ) else {
                    continue;
                };
                let status = t.column("Status").and_then(|c| c.data.get_i64(i));
                if status.is_some_and(|s| s < MIN_FIX) || !valid(lat, lon) || time < 0 {
                    continue;
                }
                // Alt is centimetres (format `e`) unless scaled to metres
                let alt_m = alt.and_then(|c| {
                    let v = c.data.get_f64(i)?;
                    Some(match c.data {
                        ColumnData::Float32(_) | ColumnData::Float64(_) => v,
                        _ => v / 100.0,
                    })
                });
                let week = t.column("GWk").and_then(|c| c.data.get_i64(i));
                let ms = t.column("GMS").and_then(|c| c.data.get_i64(i));
                let utc_us = match (week, ms) {
                    (Some(w), Some(m)) if w > 0 => {
                        Some(crate::dataflash::gps_utc_offset_us(w, m, time) + time)
                    }
                    _ => log.utc_offset_us.map(|o| o + time),
                };
                out.push(TrackPoint {
                    time_us: time as u64,
                    utc_us,
                    lat,
                    lon,
                    alt_m,
                });
            }
        }
        LogKind::Tlog => {
            let Some(t) = log.table("GPS_RAW_INT") else {
                return out;
            };
            // the vehicle that sent the first GPS_RAW_INT
            let sys = t.column("sys_id").and_then(|c| c.data.get_i64(0));
            for i in 0..t.len() {
                if sys.is_some() && t.column("sys_id").and_then(|c| c.data.get_i64(i)) != sys {
                    continue;
                }
                let (Some(time), Some(lat), Some(lon)) = (
                    t.column("timestamp_us").and_then(|c| c.data.get_i64(i)),
                    degrees(t, "lat", i),
                    degrees(t, "lon", i),
                ) else {
                    continue;
                };
                let fix = t.column("fix_type").and_then(|c| c.data.get_i64(i));
                if fix.is_some_and(|f| f < MIN_FIX) || !valid(lat, lon) || time < 0 {
                    continue;
                }
                out.push(TrackPoint {
                    time_us: time as u64,
                    utc_us: Some(time),
                    lat,
                    lon,
                    alt_m: t
                        .column("alt")
                        .and_then(|c| c.data.get_f64(i))
                        .map(|mm| mm / 1000.0),
                });
            }
        }
    }
    out
}

/// The GPS track of a PX4 ULog.
///
/// Reads `vehicle_gps_position` (PX4's selected or blended receiver), else
/// the first instance of `sensor_gps`. Both the integer fields of PX4
/// before v1.15 (`lat`, `lon` in 1e-7 degrees, `alt` in millimetres) and
/// the newer `latitude_deg`, `longitude_deg`, `altitude_msl_m` are read.
pub fn from_ulog(log: &ULog) -> Vec<TrackPoint> {
    let mut out = Vec::new();
    let Some(t) = log
        .table("vehicle_gps_position")
        .or_else(|| log.table("sensor_gps"))
        .map(primary)
    else {
        return out;
    };
    let get = |name: &str, i: usize| t.column(name).and_then(|c| c.data.get_f64(i));
    for i in 0..t.len() {
        let Some(time) = t.column("timestamp").and_then(|c| c.data.get_i64(i)) else {
            continue;
        };
        let lat = get("latitude_deg", i).or_else(|| get("lat", i).map(|v| v / 1e7));
        let lon = get("longitude_deg", i).or_else(|| get("lon", i).map(|v| v / 1e7));
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };
        let fix = t.column("fix_type").and_then(|c| c.data.get_i64(i));
        if fix.is_some_and(|f| f < MIN_FIX) || !valid(lat, lon) || time < 0 {
            continue;
        }
        let alt_m = get("altitude_msl_m", i).or_else(|| get("alt", i).map(|mm| mm / 1000.0));
        let utc_us = t
            .column("time_utc_usec")
            .and_then(|c| c.data.get_i64(i))
            .filter(|&u| u > 0);
        out.push(TrackPoint {
            time_us: time as u64,
            utc_us,
            lat,
            lon,
            alt_m,
        });
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            // characters XML 1.0 does not allow
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

/// Unix microseconds as an ISO 8601 UTC time with milliseconds
/// (`2026-11-22T10:01:02.345Z`).
pub fn iso8601_utc(unix_us: i64) -> String {
    let ms = unix_us.div_euclid(1000);
    let secs = ms.div_euclid(1000);
    let frac = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // civil from days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{frac:03}Z",
        sod / 3600,
        sod / 60 % 60,
        sod % 60
    )
}

/// The track as a GPX 1.1 document: one track, one segment, one `trkpt`
/// per sample with `ele` (metres above mean sea level) and `time` when known.
pub fn to_gpx(points: &[TrackPoint], name: &str) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"flightlog\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
    );
    s.push_str(&format!(
        "  <trk>\n    <name>{}</name>\n    <trkseg>\n",
        xml_escape(name)
    ));
    for p in points {
        s.push_str(&format!(
            "      <trkpt lat=\"{:.7}\" lon=\"{:.7}\">",
            p.lat, p.lon
        ));
        if let Some(a) = p.alt_m.filter(|a| a.is_finite()) {
            s.push_str(&format!("<ele>{a:.2}</ele>"));
        }
        if let Some(u) = p.utc_us {
            s.push_str(&format!("<time>{}</time>", iso8601_utc(u)));
        }
        s.push_str("</trkpt>\n");
    }
    s.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    s
}

/// The track as a KML 2.2 document with one `LineString`. Altitudes are
/// absolute (above mean sea level) when every point has one, else the line
/// is clamped to the ground.
pub fn to_kml(points: &[TrackPoint], name: &str) -> String {
    let with_alt = !points.is_empty()
        && points
            .iter()
            .all(|p| p.alt_m.is_some_and(|a| a.is_finite()));
    let name = xml_escape(name);
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n",
    );
    s.push_str(&format!(
        "  <Document>\n    <name>{name}</name>\n    <Placemark>\n      <name>{name}</name>\n      <LineString>\n"
    ));
    s.push_str(&format!(
        "        <altitudeMode>{}</altitudeMode>\n        <coordinates>\n",
        if with_alt {
            "absolute"
        } else {
            "clampToGround"
        }
    ));
    for p in points {
        match p.alt_m {
            Some(a) if with_alt => {
                s.push_str(&format!("          {:.7},{:.7},{a:.2}\n", p.lon, p.lat))
            }
            _ => s.push_str(&format!("          {:.7},{:.7}\n", p.lon, p.lat)),
        }
    }
    s.push_str(
        "        </coordinates>\n      </LineString>\n    </Placemark>\n  </Document>\n</kml>\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        // 2026-11-22T10:01:02.345Z
        assert_eq!(
            iso8601_utc(1_795_341_662_345_000),
            "2026-11-22T10:01:02.345Z"
        );
        assert_eq!(iso8601_utc(951_782_400_000_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601_utc(-1_000), "1969-12-31T23:59:59.999Z");
    }

    fn pts() -> Vec<TrackPoint> {
        vec![
            TrackPoint {
                time_us: 1,
                utc_us: Some(0),
                lat: -35.363_261_6,
                lon: 149.165_230_1,
                alt_m: Some(584.09),
            },
            TrackPoint {
                time_us: 2,
                utc_us: None,
                lat: -35.363_3,
                lon: 149.165_3,
                alt_m: None,
            },
        ]
    }

    #[test]
    fn gpx_document() {
        let g = to_gpx(&pts(), "a<b>&\"c\"\u{1}");
        assert!(g.contains("<name>a&lt;b&gt;&amp;&quot;c&quot;</name>"));
        assert!(g.contains("<trkpt lat=\"-35.3632616\" lon=\"149.1652301\"><ele>584.09</ele><time>1970-01-01T00:00:00.000Z</time></trkpt>"));
        assert!(g.contains("<trkpt lat=\"-35.3633000\" lon=\"149.1653000\"></trkpt>"));
        assert_eq!(g.matches("<trkpt").count(), 2);
    }

    #[test]
    fn kml_document() {
        let k = to_kml(&pts(), "f");
        assert!(k.contains("<altitudeMode>clampToGround</altitudeMode>"));
        assert!(k.contains("149.1652301,-35.3632616\n"));
        let mut p = pts();
        p[1].alt_m = Some(590.0);
        let k = to_kml(&p, "f");
        assert!(k.contains("<altitudeMode>absolute</altitudeMode>"));
        assert!(k.contains("149.1653000,-35.3633000,590.00\n"));
    }

    #[test]
    fn zero_positions_are_dropped() {
        assert!(!valid(0.0, 0.0));
        assert!(!valid(91.0, 0.0));
        assert!(!valid(f64::NAN, 1.0));
        assert!(valid(0.0, 1.0));
    }
}

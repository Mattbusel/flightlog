//! Opens any flight log (ArduPilot `.bin` / `.tlog`, PX4 `.ulg`), prints
//! its events, and writes the GPS track next to it as GPX and KML.
//!
//!     cargo run --example flight -- path/to/log.ulg

use flightlog_core::{track, FlightLog, ReadOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: flight LOG")?;
    let log = FlightLog::open(&path, &ReadOptions::default())?;
    let i = log.integrity();
    println!(
        "{path}: {}, {} tables, {} params, {} messages, {} bytes skipped",
        log.format_name(),
        log.tables().len(),
        log.params().len(),
        i.messages,
        i.skipped_bytes
    );
    let start = log.time_range_us().map(|r| r.0).unwrap_or(0);
    for e in log.events() {
        println!(
            "{:>9.3} s  {:<16} {}",
            e.time_us.saturating_sub(start) as f64 / 1e6,
            e.kind.name(),
            e.text
        );
    }
    let points = log.track();
    if !points.is_empty() {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::fs::write(format!("{path}.gpx"), track::to_gpx(&points, &name))?;
        std::fs::write(format!("{path}.kml"), track::to_kml(&points, &name))?;
        println!("{} GPS points -> {path}.gpx, {path}.kml", points.len());
    }
    Ok(())
}

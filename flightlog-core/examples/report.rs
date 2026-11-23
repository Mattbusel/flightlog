//! Prints the flight report of a log: what the flight did, why it ended, and
//! whether it broke a height ceiling.
//!
//!     cargo run --example report -- path/to/log.bin [ceiling_ft]
//!
//! With a second argument the ceiling is that many feet above the takeoff
//! point instead of the 400 ft FAA Part 107 default. Add `--json` for the
//! full machine-readable report.

use flightlog_core::report::{self, ReportOptions};
use flightlog_core::{FlightLog, ReadOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: report <log> [ceiling_ft] [--json]")?;
    let mut opts = ReportOptions::default();
    let mut json = false;
    for a in args {
        if a == "--json" {
            json = true;
        } else if let Ok(ft) = a.parse::<f64>() {
            opts.ceiling_agl_m = ft / 3.280_84;
        }
    }

    let log = FlightLog::open(&path, &ReadOptions::default())?;
    let r = report::build(&log, &opts);

    if json {
        println!("{}", r.to_json_pretty());
        return Ok(());
    }

    println!("{path}");
    println!("  SUMMARY : {}", r.summary.headline());
    println!("  ENDING  : {}", r.ending.summary);
    for e in &r.ending.evidence {
        println!("            - {:.3}s {}", e.t_s, e.detail);
    }
    println!("  ALTITUDE: {}", r.altitude.summary);
    for c in &r.altitude.ranges {
        println!(
            "            - {:.1}s to {:.1}s, peak {:.0} m",
            c.start_s, c.end_s, c.peak_m
        );
    }
    Ok(())
}

//! Prints the flight-facts JSON of a log.
//!
//! `cargo run --example facts -- path/to/00000001.BIN`

use flightlog_core::{facts, Log, ReadOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: facts <log>")?;
    let log = Log::open(&path, &ReadOptions::default())?;
    println!(
        "{}",
        facts::run(&log, &facts::Options::default()).to_json_pretty()
    );
    Ok(())
}

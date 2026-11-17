//! Prints the message types, row counts and integrity of a log.
//!
//! `cargo run --example dump -- path/to/00000001.BIN [TYPE]`

use flightlog_core::{Log, ReadOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: dump <log> [TYPE]")?;
    let t0 = std::time::Instant::now();
    let log = Log::open(&path, &ReadOptions::default())?;
    let dt = t0.elapsed();
    match args.next() {
        Some(name) => {
            let t = log.table(&name).ok_or("no such message type")?;
            if let Some(col) = args.next() {
                let c = t.column(&col).ok_or("no such column")?;
                for i in 0..c.data.len() {
                    match c.data.get_f64(i) {
                        Some(v) => println!("{v}"),
                        None => println!("{:?}", c.data.get_str(i)),
                    }
                }
                return Ok(());
            }
            println!("{} rows, columns:", t.len());
            for c in &t.columns {
                println!(
                    "  {:<12} {:<8} list={:?} unit={:?}",
                    c.name,
                    c.data.data_type().name(),
                    c.data.list_size(),
                    c.unit
                );
            }
        }
        None => {
            println!("{} ({}) read in {:?}", path, log.kind.name(), dt);
            for t in log.tables() {
                println!("  {:<12} {:>8} rows", t.name, t.len());
            }
            println!("params: {}", log.params.len());
            println!("utc offset us: {:?}", log.utc_offset_us);
            println!("integrity: {:?}", log.integrity);
        }
    }
    Ok(())
}

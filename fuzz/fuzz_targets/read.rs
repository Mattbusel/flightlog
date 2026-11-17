//! `cargo +nightly fuzz run read`: any bytes, as DataFlash and as tlog,
//! through tables, Arrow conversion and flight-facts.
#![no_main]

use flightlog_core::{facts, ReadOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let opts = ReadOptions {
        keep_order: true,
        ..ReadOptions::default()
    };
    for log in [
        flightlog_core::dataflash::read(data, &opts),
        flightlog_core::tlog::read(data, &opts),
    ] {
        for t in log.tables() {
            let b = t.to_record_batch().expect("valid arrow");
            assert_eq!(b.num_rows(), t.len());
        }
        let _ = facts::run(&log, &facts::Options::default()).to_json();
    }
});

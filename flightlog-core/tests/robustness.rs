//! Untrusted input: random bytes and mutated real logs must never panic,
//! and every byte of a DataFlash log is accounted for.

#![cfg(feature = "facts")]

use flightlog_core::dataflash::field_size;
use flightlog_core::{facts, Log, LogKind, ReadOptions};
use proptest::prelude::*;

fn mini_bin() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mini.bin")).unwrap()
}

fn mini_tlog() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/mini.tlog")).unwrap()
}

/// Bytes in decoded messages + skipped + truncated tail == file size.
fn accounted(bytes: &[u8]) {
    let log = flightlog_core::dataflash::read(bytes, &ReadOptions::default());
    let mut used = 0u64;
    for t in log.tables() {
        let fmt = t.format.as_deref().unwrap();
        let body: usize = fmt.bytes().map(|c| field_size(c).unwrap()).sum();
        used += t.len() as u64 * (body as u64 + 3);
    }
    let i = &log.integrity;
    assert_eq!(
        used + i.skipped_bytes + i.truncated_tail_bytes,
        bytes.len() as u64,
        "{i:?}"
    );
}

fn exercise(bytes: &[u8]) {
    let opts = ReadOptions {
        keep_order: true,
        ..ReadOptions::default()
    };
    let df = flightlog_core::dataflash::read(bytes, &opts);
    let tl = flightlog_core::tlog::read(bytes, &opts);
    for log in [&df, &tl] {
        let doc = facts::run(log, &facts::Options::default());
        let _ = doc.to_json();
        // what the schema requires of numbers the checks produce
        for c in &doc.checks {
            for t in &c.thresholds {
                assert!(t.value.is_finite(), "{} {}", c.id, t.name);
            }
            for r in &c.ranges {
                assert!(r.start_s.is_finite() && r.end_s.is_finite(), "{}", c.id);
            }
        }
        assert!(doc.log.duration_s.is_finite() && doc.log.duration_s >= 0.0);
        if let Some((lo, hi)) = log.time_range_us() {
            assert!(lo <= hi);
        }
        #[cfg(feature = "arrow")]
        for t in log.tables() {
            let b = t.to_record_batch().unwrap();
            assert_eq!(b.num_rows(), t.len());
        }
        let order = log.order.as_ref().unwrap();
        for &(t, r) in order {
            assert!((r as usize) < log.tables()[t as usize].len());
        }
    }
    let _ = Log::read(bytes, &ReadOptions::default());
}

#[test]
fn committed_fixtures_are_accounted() {
    accounted(&mini_bin());
    let log = Log::read(&mini_bin(), &ReadOptions::default()).unwrap();
    assert_eq!(log.kind, LogKind::DataFlash);
    assert_eq!(log.integrity.skipped_bytes, 0);
    let log = Log::read(&mini_tlog(), &ReadOptions::default()).unwrap();
    assert_eq!(log.kind, LogKind::Tlog);
}

#[test]
fn empty_and_tiny() {
    for n in 0..16 {
        exercise(&vec![0xA3; n]);
        exercise(&vec![0u8; n]);
        accounted(&vec![0x95; n]);
    }
    assert!(Log::read(&[], &ReadOptions::default()).is_err());
}

#[test]
fn every_truncation_of_the_head() {
    let b = mini_bin();
    for n in (0..4096).chain((4096..b.len()).step_by(997)) {
        accounted(&b[..n]);
    }
}

#[test]
fn type_filter_keeps_counts() {
    let b = mini_bin();
    let all = Log::read(&b, &ReadOptions::default()).unwrap();
    let mut types = std::collections::HashSet::new();
    types.insert("PARM".to_string());
    let some = Log::read(
        &b,
        &ReadOptions {
            types: Some(types),
            ..ReadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(some.tables().len(), 1);
    assert_eq!(some.counts, all.counts);
    assert_eq!(some.params, all.params);
    assert_eq!(some.table("PARM"), all.table("PARM"));
}

#[test]
fn order_replays_the_file() {
    let b = mini_bin();
    let log = Log::read(
        &b,
        &ReadOptions {
            keep_order: true,
            ..ReadOptions::default()
        },
    )
    .unwrap();
    let order = log.order.as_ref().unwrap();
    assert_eq!(order.len() as u64, log.integrity.messages);
    // rows of each table appear in order 0, 1, 2, ...
    let mut next = vec![0u32; log.tables().len()];
    for &(t, r) in order {
        assert_eq!(r, next[t as usize]);
        next[t as usize] += 1;
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        exercise(&bytes);
        accounted(&bytes);
    }

    #[test]
    fn random_bytes_with_headers(chunks in proptest::collection::vec((any::<u8>(), proptest::collection::vec(any::<u8>(), 0..100)), 0..64)) {
        // biased toward DataFlash headers and FMT messages
        let mut bytes = Vec::new();
        for (t, body) in chunks {
            bytes.extend_from_slice(&[0xA3, 0x95, if t < 64 { 0x80 } else { t }]);
            bytes.extend_from_slice(&body);
        }
        exercise(&bytes);
        accounted(&bytes);
    }

    #[test]
    fn mutated_dataflash(edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>(), 0u8..4), 1..32)) {
        let mut b = mini_bin();
        for (at, v, kind) in edits {
            if b.is_empty() { break; }
            let i = at.index(b.len());
            match kind {
                0 => b[i] = v,
                1 => b[i] ^= 0xFF,
                2 => { b.remove(i); }
                _ => b.insert(i, v),
            }
        }
        exercise(&b);
        accounted(&b);
    }

    #[test]
    fn mutated_tlog(edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>(), 0u8..4), 1..32)) {
        let mut b = mini_tlog();
        b.truncate(40_000);
        for (at, v, kind) in edits {
            if b.is_empty() { break; }
            let i = at.index(b.len());
            match kind {
                0 => b[i] = v,
                1 => b[i] ^= 0xFF,
                2 => { b.remove(i); }
                _ => b.insert(i, v),
            }
        }
        exercise(&b);
    }
}

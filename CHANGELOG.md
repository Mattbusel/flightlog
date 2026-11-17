# Changelog

## 0.1.0 (2026-11-17)

First release.

- `flightlog-core` 0.1.0 (crates.io): DataFlash `.bin` and MAVLink `.tlog`
  readers into typed columnar tables; pymavlink-compatible values and damaged
  log recovery with full byte accounting (`Integrity`); `ReadOptions` type
  filter, raw (unscaled) mode and file-order replay; `arrow` feature
  (`Table::to_record_batch`); `facts` feature (flight-facts v1).
- flight-facts v1 JSON Schema with 12 checks: `vibration.levels`,
  `vibration.clipping`, `vibration.failsafe`, `ekf.variance`, `gps.hdop`,
  `gps.satellites`, `gps.glitch`, `battery.voltage`, `power.board_vcc`,
  `power.log_ends_armed`, `rc.failsafe`, `link.gcs_failsafe`, each quoting
  the ArduPilot wiki (ardupilot_wiki 3217b658).
- `flightlog` 0.1.0 Python wheel (abi3, CPython 3.9+): `Log` with pyarrow,
  polars, pandas and Parquet output, `facts()`, `flightlog.compat.DFReader_binary`,
  `flightlog` CLI and the read-only `flightlog-mcp` server.
- Tested against pymavlink 2.4.50 on 19 ArduPilot SITL autotest logs and
  damaged copies (2,686,532 messages); property tests and a fuzz target for
  untrusted input.

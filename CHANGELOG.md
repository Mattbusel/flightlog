# Changelog

## 0.1.1 (2026-11-17)

Fixes from an independent review of 0.1.0. flight-facts is now 1.1; a 1.0
document is valid against the 1.1 schema.

- Damaged timestamps no longer stretch a log. `Log::time_range_us` keeps
  only timestamps that have two others within 2 s and are in order in their
  own table; `Log::time_outliers` (Python `integrity["time_outliers"]`,
  flight-facts `integrity.time_outliers`) counts the rest. flight-facts
  leaves those rows out of checks, events and armed ranges. On
  `copter-mission-bitflip.bin` the duration was 18,374,686,479,873 s and is
  now 240.18 s, the same as the undamaged log.
- `Log.to_parquet` and `flightlog export` no longer write outside the target
  directory: message names (from the log's FMT records) are escaped to safe
  file names (`flightlog.parquet_file_name`).
- flight-facts no longer applies Copter rules and defaults to other
  vehicles. `vibration.failsafe` is `not_applicable` for them, and the Copter
  wiki defaults for `FS_EKF_THRESH`, `BATT_LOW_VOLT`, `FS_GCS_TIMEOUT` and
  `RC_FS_TIMEOUT` are used only for Copter and Heli logs. Without the log's
  own value, `ekf.variance` and `battery.voltage` are `not_applicable`. On a
  Copter log without `BATT_LOW_VOLT`, a long dip below the 10.5 V default is
  `warn`, not `fail`.
- `rc.failsafe` and `link.gcs_failsafe` read the failsafe texts (DataFlash
  `MSG`) when a log has no `ERR` for them, as Plane and Rover logs do.
  Failsafe texts are listed in DataFlash events too.
- `ekf.variance` applies the failsafe rule to the primary EKF core (`PI`,
  else core 0); other cores give `warn`.
- A parameter logged as `NaN` or infinity is treated as missing
  (`params_used` origin `param_invalid`); before, it made a silent `pass` and
  a document that failed the schema.
- `power.log_ends_armed` and the `disarmed` event: a disarm logged at the last
  timestamp no longer counts as ending armed.
- `Integrity::is_clean` and flight-facts `integrity.clean` have one
  definition: nothing skipped, no bad checksum, no rejected format. A cut-off
  final message does not count.
- `Log.table(name, instance=k)` raises `ValueError` when the message type has
  no instance field, instead of returning every row.
- Container detection: a tlog is recognized by the checksum of its first
  frame and DataFlash by `0xA3 0x95 0x80`; `Log::read` returns
  `UnknownFormat` when no message decodes (a text file with `0xFD` at byte 8
  used to open as an empty tlog).
- flightlog-mcp: errors the caller can fix (unknown message, field or check,
  path outside the roots) reach the client with their message (`ToolError`);
  mcp 2 hid them.
- Tests: an ArduPlane SITL log (`plane-throttle-failsafe.bin`, equal to
  pymavlink 2.4.50), synthetic DataFlash logs for the cases above, a
  real-time synthetic tlog for the time-based tlog rules (the autotest tlogs
  run at SIM_SPEEDUP 100), and schema validation in the hypothesis tests.
- Docs: install from the GitHub release until the package is on PyPI.

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

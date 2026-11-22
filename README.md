# flightlog: the log from the crash still opens

**One flipped byte makes other readers give up on the whole flight. flightlog drops the one damaged message, keeps the other 134,878, and tells you exactly which bytes it skipped.**

```text
$ flightlog copter-mission-bitflip.bin     # 64 bytes flipped; pymavlink refuses to open this file

  read         134,878 of 134,976 messages   (one damaged message type dropped)
  integrity    64 bytes unusable, every offset recorded

  flight-facts: vibration, clipping, EKF variance, GPS, battery, board
  voltage, RC and GCS failsafe. Each verdict quotes the ArduPilot wiki
  sentence its threshold comes from, with the exact time ranges involved.
```

[**Live demo**](https://mattbusel.github.io/drone-demos/)
&nbsp;·&nbsp;
[![crates.io](https://img.shields.io/crates/v/flightlog-core.svg)](https://crates.io/crates/flightlog-core)
[![downloads](https://img.shields.io/crates/d/flightlog-core.svg)](https://crates.io/crates/flightlog-core)
[![docs.rs](https://img.shields.io/docsrs/flightlog-core)](https://docs.rs/flightlog-core)
[![license](https://img.shields.io/crates/l/flightlog-core.svg)](#license)

flightlog reads ArduPilot DataFlash (`.bin`), MAVLink (`.tlog`) and PX4 ULog (`.ulg`) logs into typed columnar tables (Arrow optional), one table per message type. It is checked field by field against the tools people already trust: equal to pymavlink on 2,686,532 messages across 19 undamaged logs, and equal to pyulog on all five of its own test logs, down to every row count, field sum and parameter. On damaged logs it does not stop where those tools stop; it recovers what is readable and accounts for every byte it could not use. It is `no_std`-friendly at the core, has no required C dependency, and every flight-facts verdict quotes the ArduPilot wiki sentence its threshold comes from, so a check is never a number you have to trust on faith.

---

## The one that matters: the log from the crash

Log readers that index the whole file before reading it give up on a single
bad byte in a format record. Flip 64 bytes in a 135,000-message ArduPilot log
and pymavlink 2.4.50, the Python reader ArduPilot's own tools are built on,
refuses to open the file at all (`Unsupported format char`). Zero messages, at
the moment you need all of them. flightlog opens it, drops the one damaged
message type, and keeps 134,878 of the 134,976 messages:

| | pymavlink 2.4.50 | flightlog |
|---|---|---|
| `copter-mission-bitflip.bin` (64 bytes flipped) | does not open | **134,878 of 134,976 messages** |
| 19 undamaged SITL logs, 2,686,532 messages | reference | **every count and field fingerprint equal** |
| CRC-valid frames next to console text (`copter-motor-vibration.tlog`) | 19 missed by its type-filtered read | **read** |
| 32 MB `.bin`, every message | 3.49 s | **0.13 s** |
| Bytes it could not use | not reported | every one accounted for (`Integrity`) |

The checks do not ask you to trust them either: every flight-facts verdict
quotes the ArduPilot wiki sentence its threshold comes from.

## What you get

- **PX4 ULog** (`.ulg`) into the same tables, equal to pyulog on its own
  test logs, with GPX/KML track export and mode, arming, failsafe and
  failure-detector events for both autopilots.
- **Arrow tables** from DataFlash `.bin` and MAVLink `.tlog` logs: one table
  per message type, values identical to pymavlink's, damaged logs recovered
  and accounted for byte by byte. Rust crate `flightlog-core`, Python wheel
  `flightlog` (pyarrow, polars, pandas, Parquet).
- **flight-facts v1**: a JSON Schema and a deterministic set of log checks
  (vibration, clipping, EKF variance, GPS, battery, board voltage, RC and GCS
  failsafes). Each check quotes the ArduPilot wiki sentence its threshold
  comes from and gives the time ranges involved.
- **flightlog-mcp**: a read-only MCP server so an assistant can read the facts
  and the rows behind them.

## Install

`cargo add flightlog-core`; Python: `pip install flightlog`.
Until `flightlog` is on PyPI, install the wheel from the GitHub release:
`pip install flightlog --find-links https://github.com/Mattbusel/flightlog/releases/expanded_assets/v0.1.2`
(add `"flightlog[mcp]"` or `"flightlog[polars]"` the same way).

## Use it

```python
import flightlog

log = flightlog.open("00000001.BIN")
log.table("ATT")                      # pyarrow.Table
for c in log.facts()["checks"]:
    print(c["id"], c["status"], c["summary"])
```

```rust
use flightlog_core::{facts, Log, ReadOptions};

let log = Log::open("00000001.BIN", &ReadOptions::default())?;
let roll = log.table("ATT").unwrap().f64s("Roll").unwrap();
let doc = facts::run(&log, &facts::Options::default());
```

## PX4 ULog, GPS tracks, flight events

The same tables for PX4: `ULog::open("log_0.ulg")` reads every topic
(nested types and arrays flattened with pyulog's column names, one
`multi_id` column per table), the parameters at start plus every change in
flight, the system and airframe defaults, console messages and the
logger's own dropout records. `FlightLog::open` takes a `.bin`, `.tlog` or
`.ulg` and answers the same calls for all three, including the GPS track
(GPX or KML out) and the flight's events.

```rust
use flightlog_core::{track, FlightLog, ReadOptions};

let log = FlightLog::open("log_0.ulg", &ReadOptions::default())?; // or .bin / .tlog
for e in log.events() {
    println!("{} {} {}", e.time_us, e.kind.name(), e.text);
}
std::fs::write("flight.gpx", track::to_gpx(&log.track(), "log_0"))?;
```

`cargo run --example flight -- sample_px4_events.ulg` on pyulog's test log:

```text
    0.072 s  mode             Hold
    1.860 s  message          [WARNING] [health_and_arming_checks] Preflight: GPS fix too low
   15.228 s  mode             Takeoff
   15.228 s  armed            armed
   21.588 s  mode             Hold
   22.428 s  mode             Return
   30.432 s  mode             Hold
   30.432 s  disarmed         disarmed
423 GPS points -> sample_px4_events.ulg.gpx, sample_px4_events.ulg.kml
```

| | pyulog 1.3.0 | flightlog |
|---|---|---|
| pyulog's 5 test logs, 203,360 data rows | reference | **every topic, row count, field sum and parameter equal** |
| `sample.ulg` with 64 bytes flipped | 64,430 rows, `file_corruption = True` | **64,430 rows, 516 skipped bytes with their offsets** |
| 4 MB `sample.ulg`, best of 10 / 5 runs, one Windows PC | 0.054 s | **0.005 s** |

pyulog tells you a damaged file is damaged. flightlog tells you which bytes
it could not use, so you know whether the gap is in the seconds you care
about. Events read `vehicle_status` (mode, arming, failsafe, failure
detector) and warnings from the console log; GPS tracks keep 3D fixes only
and drop the 0, 0 positions receivers report before their first fix.

## Docs

[reading](docs/reading.md) · [flight-facts](docs/flight-facts.md) ·
[Python](docs/python.md) · [MCP server](docs/mcp.md) · [testing](docs/testing.md)

## License

MIT licensed.

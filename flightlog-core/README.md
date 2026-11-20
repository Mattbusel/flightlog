# flightlog: the log from the crash still opens

**Your log tool dies on the one log you need: the one from the crash.**
Log readers that index the whole file before reading it give up on a
single bad byte in a format record. Flip 64 bytes in a 135,000-message
ArduPilot log and pymavlink 2.4.50, the Python reader ArduPilot's own tools
are built on, refuses to open the file at all (`Unsupported format char`).
Zero messages, at the moment you need all of them. flightlog opens it, drops the one damaged
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

ArduPilot flight logs as tables, plus checks you can cite.

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

Install: `cargo add flightlog-core`; Python: `pip install flightlog`.
Until `flightlog` is on PyPI, install the wheel from the GitHub release:
`pip install flightlog --find-links https://github.com/Mattbusel/flightlog/releases/expanded_assets/v0.1.2`.

Docs: [reading](https://github.com/Mattbusel/flightlog/blob/main/docs/reading.md) · [flight-facts](https://github.com/Mattbusel/flightlog/blob/main/docs/flight-facts.md) ·
[Python](https://github.com/Mattbusel/flightlog/blob/main/docs/python.md) · [MCP server](https://github.com/Mattbusel/flightlog/blob/main/docs/mcp.md) · [testing](https://github.com/Mattbusel/flightlog/blob/main/docs/testing.md)

MIT licensed.

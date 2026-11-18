# flightlog

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
`pip install flightlog --find-links https://github.com/Mattbusel/flightlog/releases/expanded_assets/v0.1.1`.

Docs: [reading](https://github.com/Mattbusel/flightlog/blob/main/docs/reading.md) · [flight-facts](https://github.com/Mattbusel/flightlog/blob/main/docs/flight-facts.md) ·
[Python](https://github.com/Mattbusel/flightlog/blob/main/docs/python.md) · [MCP server](https://github.com/Mattbusel/flightlog/blob/main/docs/mcp.md) · [testing](https://github.com/Mattbusel/flightlog/blob/main/docs/testing.md)

MIT licensed.

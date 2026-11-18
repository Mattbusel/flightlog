# flightlog

ArduPilot DataFlash (`.bin`) and MAVLink telemetry (`.tlog`) logs as Arrow
tables, plus **flight-facts**: deterministic log checks whose thresholds are
quoted from the ArduPilot wiki, and a read-only MCP server.

```text
pip install flightlog            # tables and facts (needs pyarrow)
pip install "flightlog[mcp]"     # plus the flightlog-mcp server
```

Until `flightlog` is on PyPI, install the wheel from the GitHub release:
`pip install flightlog --find-links https://github.com/Mattbusel/flightlog/releases/expanded_assets/v0.1.1`
(add `"flightlog[mcp]"` or `"flightlog[polars]"` the same way).

```python
import flightlog

log = flightlog.open("00000001.BIN")
att = log.table("ATT")                 # pyarrow.Table, one row per message
imu = log.polars("IMU", instance=0)    # polars.DataFrame
print(log.params["FS_EKF_THRESH"])
for c in log.facts()["checks"]:
    print(c["id"], c["status"], c["summary"])
```

- Values match pymavlink's `DFReader` (scaled `c/C/e/E/L` fields, text cut
  at NUL); tested message for message against pymavlink 2.4.50 on ArduPilot
  SITL autotest logs and on damaged copies of them.
- Damaged logs: the reader resynchronizes like pymavlink and reports every
  skipped byte range in `log.integrity`.
- `flightlog.compat.DFReader_binary` gives pymavlink's `recv_match()` style
  on top of the same reader.
- `flightlog-mcp` serves logs to MCP clients (read-only, limited to the
  directories you pass).

Documentation: https://github.com/Mattbusel/flightlog/tree/main/docs

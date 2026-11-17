# Python

```text
pip install flightlog              # needs pyarrow >= 14
pip install "flightlog[polars]"    # Log.polars()
pip install "flightlog[mcp]"       # flightlog-mcp
```

One abi3 wheel per platform covers CPython 3.9 and later.

## Tables

```python
import flightlog

log = flightlog.open("00000001.BIN")       # .bin or .tlog, detected from content
log.kind                                   # "dataflash" or "tlog"
log.message_types                          # ["FMT", "PARM", ..., "ATT", ...]
log.counts["ATT"]                          # messages per type
log.columns("GPS")                         # [(name, arrow type, unit), ...]
log.instance_field("IMU")                  # "I" (from FMTU), or None

att = log.table("ATT")                     # pyarrow.Table
imu1 = log.table("IMU", instance=1)        # rows of one instance
gps = log.polars("GPS")                    # polars.DataFrame
bat = log.pandas("BAT")                    # pandas.DataFrame

att.schema.field("Roll").metadata          # {b"unit": b"deg"}
att.schema.metadata[b"flightlog.format"]   # DataFlash format string

log.params["FS_EKF_THRESH"]                # 0.8
log.integrity                              # skipped bytes and regions, truncated tail, bad CRC, rejected formats
log.utc_offset_us                          # Unix microseconds at TimeUS 0 (from GPS time)
log.time_range_us                          # (first, last) in the log's time base

log.to_parquet("out/")                     # one Parquet file per message type
```

Options: `flightlog.open(path, types=["ATT", "GPS"])` builds only those
tables (counts and parameters stay complete); `scale=False` keeps DataFlash
`c/C/e/E/L` fields as raw integers; `keep_order=True` records file order for
`log.order()`. `flightlog.from_bytes(data)` reads from memory, and
`flightlog.read_table(path, "ATT")` reads one type.

Reading releases the GIL, so several logs can be read in threads.

## flight-facts

```python
facts = log.facts()                         # dict, flight-facts v1
for c in facts["checks"]:
    print(c["id"], c["status"], c["summary"])
    for r in c["ranges"]:
        print("   ", r["start_s"], r["end_s"], r["detail"])

flightlog.facts_schema()                    # the JSON Schema, as text
```

See [flight-facts.md](flight-facts.md) for every rule.

## pymavlink-style reading

```python
from flightlog.compat import DFReader_binary

mlog = DFReader_binary("00000001.BIN")
while (m := mlog.recv_match(type=["GPS", "XKF4"], condition="GPS.NSats > 6")) is not None:
    print(m.get_type(), m._timestamp, m.to_dict())
```

`DFReader_binary` replays the log in file order with pymavlink's message
interface: `recv_msg`, `recv_match(type, condition)`, `rewind`, `params`,
`param()`, `messages` (including `NAME[instance]` keys), and messages with
attribute access, `get_type()`, `get_fieldnames()`, `to_dict()`,
`_timestamp`, `fmt` and `_elements` (unscaled values). Tests replay a SITL log
through both side by side; see [testing.md](testing.md). It opens logs that
make pymavlink raise during indexing, and `integrity` says what was skipped.

## Command line

```text
flightlog info LOG              message types, counts, integrity
flightlog facts LOG [--pretty]  flight-facts JSON (exit status 1 if any check fails)
flightlog export LOG DIR        Parquet per message type
flightlog schema                the flight-facts JSON Schema
```

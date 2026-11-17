# Reading logs

`flightlog-core` reads two containers into one table per message type:

| | DataFlash `.bin` | MAVLink `.tlog` |
|---|---|---|
| Written by | the autopilot (onboard log) | the ground station (QGroundControl, Mission Planner, MAVProxy) |
| Message definitions | in the log (FMT, FMTU, UNIT, MULT) | MAVLink `all.xml` dialect, built in (353 messages) |
| Time column | `TimeUS` (microseconds since boot) | `timestamp_us` (Unix microseconds, as logged) |
| Extra columns | none | `sys_id`, `comp_id` |
| Parameters (`Log::params`) | PARM | PARAM_VALUE |

The container is detected from the content (`LogKind::detect`), not the file
name.

## Values

DataFlash values are those pymavlink's `DFReader` returns:

| Format chars | Arrow type |
|---|---|
| `b B h H i I q Q` | int8, uint8, int16, uint16, int32, uint32, int64, uint64 |
| `M` (flight mode) | uint8 |
| `f`, `d` | float32, float64 |
| `g` (half float) | float32 |
| `c C e E` | float64 = raw / 100 (raw integers with `scale=False`) |
| `L` (lat/lon) | float64 = raw / 1e7 degrees (raw int32 with `scale=False`) |
| `n N Z` | utf8: UTF-8 if the whole field decodes, else Latin-1, cut at the first NUL |
| `Z` in `FILE` | binary |
| `a` | fixed_size_list<int16>[32] |

pymavlink reads `M` as signed; flight mode numbers are below 128, so the
values are the same.

Units come from FMTU/UNIT/MULT and are shown the way pymavlink shows them
(`"m/s/s"`, `"cm"`, `"1e-07 deglatitude"`); in Arrow they are field metadata
(`unit`, plus `multiplier` for the raw MULT value). The schema metadata holds
the message name, the instance field (the column FMTU marks with `#`) and
the format string.

tlog fields follow the dialect definition in XML order; numeric arrays are
fixed-size lists, `char[]` fields are text up to the first NUL. MAVLink 2
frames with trimmed trailing zeros, and MAVLink 1 frames without extension
fields, read as zeros in the missing bytes, as in pymavlink.

## Damaged logs

DataFlash has no checksums, so recovery follows pymavlink's sequential parser
(`DFReader_binary._parse_next`) byte for byte:

- a message starts at `0xA3 0x95 <type>` where `<type>` has been defined;
  anything else is skipped one byte at a time;
- a message cut off by the end of the file is dropped;
- a FMT record with an unknown format character is dropped and reading
  resumes right after its header; messages of a type whose format does not
  add up to its length are skipped the same way.

Every skipped byte is accounted for: decoded messages + `skipped_bytes` +
`truncated_tail_bytes` equals the file size (a property test checks this on
random and mutated input). `Integrity` lists the skipped runs (the first 256),
rejected formats and their reasons.

One difference matters in practice. pymavlink's `DFReader_binary` indexes the
whole file when it opens it, and that index step raises on a FMT record with
an unknown format character, so the log does not open at all. In the test
corpus, `copter-mission-bitflip.bin` (64 inverted bytes, one of them in the
`XKT` format string) makes pymavlink 2.4.50 raise `Unsupported format char`;
flightlog reads it, drops the `XKT` type, and keeps 134,878 of the 134,976
messages the undamaged log has. ArduPilot's MethodicConfigurator validates
every log in a child process before opening it, because pymavlink's optional
Cython indexer exits the process on malformed FMT records.

tlog frames carry a CRC: flightlog uses `mavlink-tlog`, which checks the CRC
of every known message and resynchronizes one byte at a time after damage.
On the autotest tlogs, where SITL console text is mixed into the file,
pymavlink drops some valid frames next to that text, in its sequential
`recv_msg()` and in its type-filtered `recv_match(type=...)`. In
`copter-motor-vibration.tlog` flightlog returns 19 CRC-valid frames (15
PARAM_VALUE, 3 STATUSTEXT, 1 AHRS) that pymavlink's type-filtered pass does
not. On a cleaned copy with the console text removed
(`flightlog-core/examples/tlog_clean.rs`), pymavlink and flightlog return the same 36,583
frames with the same values.

## Time

`Log::utc_offset_us` gives Unix time at `TimeUS == 0` for DataFlash logs,
from the first `GPS` message with a week number, using pymavlink's rule (GPS
epoch 1980-01-06, 18 leap seconds). `Log::time_range_us` ignores `TimeUS == 0`
rows written before the clock starts.

## Selecting types and replaying

`ReadOptions::types` limits which tables are built; every message is still
counted, and FMT/FMTU/UNIT/MULT/PARM are always interpreted, so units and
parameters stay complete. `ReadOptions::keep_order` records `(table, row)` for
every message in file order (`Log::order`), which is what
`flightlog.compat.DFReader_binary` replays.

## Why not an existing Rust reader

`ardupilot-binlog` 0.2.0 (MIT/Apache) and `dflog` 0.2.0 exist. The plan was to
evaluate `ardupilot-binlog` as the `.bin` dependency. It was not used:

- it is row oriented (a `Vec<FieldValue>` per message), while flightlog needs
  to fill typed columns without a per-row allocation;
- it stops after 256 consecutive errors, so a long damaged stretch ends the
  read, where pymavlink and flightlog keep scanning;
- it returns `L` (lat/lon) unscaled and has no FMTU/UNIT/MULT handling, so
  matching pymavlink's values would need a second layer anyway.

The DataFlash reader here is about 400 lines and is tested against pymavlink
message for message (see [testing.md](testing.md)).

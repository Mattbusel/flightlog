# Testing

```text
bash tools/fetch_testdata.sh                          # SITL logs, sha256-checked, plus damaged copies
cargo test -p flightlog-core --release --all-features
cargo test -p flightlog-core --no-default-features
cd python && maturin develop --release && pytest tests
python tools/check_wiki_quotes.py --latest            # wiki quotes still verbatim
cargo +nightly fuzz run read                          # libFuzzer, from fuzz/
```

Without the downloaded logs the reference and SITL tests print `SKIP` and
pass; the committed fixtures (`flightlog-core/tests/data/mini.bin`,
`mini.tlog`) are always tested.

## Test logs

All logs are from ArduPilot SITL, ArduCopter `V4.8.0-dev`, ardupilot commit
`ca137a86cf2862a14b13dcddde18560094abbec4`, built with `./waf copter` and run
with `Tools/autotest/autotest.py test.Copter.<Test>` on 2026-11-17. Each
autotest injects one fault, which makes the logs useful for checking the
flight-facts rules:

| File | Autotest | What it contains |
|---|---|---|
| `copter-mission.bin`, `.tlog` | `CopterMission` | a normal AUTO mission |
| `copter-throttle-failsafe.bin` | `ThrottleFailsafe` | radio failsafes, late RC frames, EKF failsafes |
| `copter-gcs-failsafe.bin`, `.tlog` | `GCSFailsafe` | GCS failsafes |
| `copter-battery-failsafe.bin` | `BatteryFailsafeTwoStage` | battery failsafe |
| `copter-vibration-failsafe.bin`, `.tlog` | `VibrationFailsafe` | vibration compensation, a GPS glitch |
| `copter-gps-glitch.bin`, `.tlog` | `GPSGlitchLoiter` | GPS glitch |
| `copter-motor-vibration.tlog` | `MotorVibration` | SITL console text mixed into the tlog |
| `*-clean.tlog` | | the tlogs with the console text removed (`flightlog-core/examples/tlog_clean.rs`) |

They are attached to the `testdata-v1` GitHub release; `testdata/SHA256SUMS`
pins them. `tools/make_damaged.py` derives four damaged copies of
`copter-mission.bin` with a fixed-seed generator: cut mid-message
(`-truncated`), two 4 KiB zero runs (`-zeroblock`), 64 inverted bytes
(`-bitflip`) and 1000 inserted bytes (`-garbage`).

## Reference: pymavlink

`testdata/*.pymavlink.json` come from `tools/pymavlink_reference.py` with
pymavlink 2.4.50: per message type the count, and per field a fingerprint of
all values in file order (exact integer sum, float sum in file order, CRC-32
of text). `flightlog-core/tests/reference.rs` computes the same from
flightlog's tables and requires equality.

Result on 2026-11-17: 19 logs, 2,686,532 messages, every count and every
field fingerprint equal, with two documented exceptions:

- `copter-mission-bitflip.bin` has no reference: pymavlink 2.4.50 raises
  `Unsupported format char` when opening it (see [reading.md](reading.md)).
  `tests/facts.rs::bitflip_log_recovers` checks what flightlog recovers.
- `copter-motor-vibration.tlog`: flightlog returns 19 more CRC-valid frames
  than pymavlink; the test allows more rows, never fewer, for tlogs, and the
  clean copy is compared exactly.

The Python tests also replay `copter-mission.bin` through
`flightlog.compat.DFReader_binary` and pymavlink's `DFReader_binary` side by
side: all 134,976 messages have the same type, the same `to_dict()` and the
same `_timestamp` (within 1 microsecond) for every message with a `TimeUS`
field, and `recv_match(type=..., condition=...)`
returns the same messages with the same `_elements`.

## Untrusted input

- `tests/robustness.rs` (proptest): random bytes, random bytes shaped like
  DataFlash headers and FMT records, and edits (overwrite, invert, delete,
  insert) of the committed logs go through both readers, Arrow conversion and
  flight-facts without a panic; every byte of a DataFlash input is accounted
  for; every truncation of the first 4 KiB is read.
- `python/tests` (hypothesis): random and mutated bytes through the wheel.
- `fuzz/` (cargo-fuzz): the same path under libFuzzer. CI runs it for 60 s.

## Speed

One measurement, not a benchmark suite: `copter-throttle-failsafe.bin`
(32,477,184 bytes, 783,602 messages), Windows 11, Intel family 6 model 183,
Python 3.12.10, release build, 2026-11-17:

| | time |
|---|---|
| `flightlog.open()` (all tables) | 0.128 s to 0.139 s (3 runs) |
| all tables to `pyarrow.Table` | 0.029 s to 0.117 s (3 runs) |
| `flightlog.read_table(path, "ATT")` | 0.104 s |
| pymavlink `DFReader_binary` open plus `recv_msg()` loop | 3.49 s (1 run) |
| pymavlink `recv_match(type="ATT")` loop | 0.25 s (1 run) |

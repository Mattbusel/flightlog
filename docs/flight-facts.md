# flight-facts v1

flight-facts is a JSON document of deterministic checks over one ArduPilot
log. The point is a shared ground truth under log diagnosis, human or LLM:
every number comes from the log, every threshold comes from the ArduPilot
wiki (quoted verbatim, with the wiki commit it was checked at) or from the
vehicle parameter the wiki names, and every finding carries the time ranges
it refers to. The same log always produces the same document.

The example values below are from `copter-mission.bin` (ArduPilot SITL
autotest `CopterMission`, see [testing.md](testing.md)).

- Schema: [`flightlog-core/schemas/flight-facts-v1.schema.json`](../flightlog-core/schemas/flight-facts-v1.schema.json)
  (JSON Schema 2020-12; also `flightlog.facts_schema()` and `flightlog schema`).
- Producers: `flightlog_core::facts::run` (Rust), `Log.facts()` (Python),
  `flightlog facts LOG` (CLI), the `flight_facts` MCP tool.
- The wiki quotes are checked in CI by `tools/check_wiki_quotes.py` against
  the pinned ardupilot_wiki commit `3217b658a9fa3bb5c766d320eac5b447651b48f9`
  and against master. Whitespace is compared collapsed, because the wiki
  source wraps lines.

## Document

```json
{
  "$schema": "https://raw.githubusercontent.com/Mattbusel/flightlog/main/flightlog-core/schemas/flight-facts-v1.schema.json",
  "flight_facts_version": "1.1",
  "generator": {"name": "flightlog", "version": "0.1.1"},
  "log": {"kind": "dataflash", "vehicle": "copter", "firmware": "ArduCopter V4.8.0-dev (ca137a86)",
          "time_base": "boot_us", "start_us": 2459849, "end_us": 242639572, "duration_s": 240.18,
          "utc_start": "2026-11-17T22:06:24.260Z", "message_types": 74, "messages": 134976},
  "integrity": {"bytes": 5595136, "skipped_bytes": 0, "skipped_regions": 0, "truncated_tail_bytes": 6,
                "bad_crc": 0, "invalid_formats": [], "time_outliers": 0, "clean": true},
  "armed": {"source": "ARM.ArmState", "total_s": 195.73, "ranges": [...]},
  "params_used": {"FS_EKF_THRESH": {"value": 0.8, "origin": "param"}, ...},
  "summary": {"pass": 10, "info": 0, "warn": 1, "fail": 0, "not_applicable": 1},
  "checks": [
    {"id": "gps.satellites", "status": "warn",
     "summary": "Satellite count fell to 10, below the 12 the wiki calls bad to fall below.",
     "thresholds": [{"name": "warn_below", "value": 12.0, "unit": "satellites", "origin": "wiki"}],
     "measured": {"max": 10.0, "min": 10.0, "seconds_below_12": 195.600062},
     "ranges": [{"start_us": 43599220, "end_us": 239199282, "start_s": 41.139, "end_s": 236.739,
                 "detail": "fewer than 12 satellites"}],
     "evidence": ["GPS.HDop", "GPS.NSats", "GPS.Status"],
     "sources": [{"url": "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
                  "quote": "Hdop values below 1.5 are very good, values over 2.0 could indicate the GPS positions are not good.  The number of satellites falling below 12 is also bad.",
                  "wiki_file": "common/source/docs/common-diagnosing-problems-using-logs.rst",
                  "wiki_commit": "3217b658a9fa3bb5c766d320eac5b447651b48f9"}]}
  ],
  "events": [{"time_us": 2539817, "t_s": 0.08, "kind": "error", "text": "Compass: Errors Resolved", "code": 3, "subcode": 0}, ...]
}
```

Times: `*_us` values are in the log's own time base (`boot_us` = DataFlash
TimeUS, `unix_us` = tlog record time). `start_s`, `end_s` and `t_s` are
seconds from the first message of the log, which is what a person scrubbing
a plot wants.

Status values: `pass` (within the wiki's thresholds), `info` (worth knowing,
still within what the wiki calls acceptable), `warn` (the wiki says it may be
a problem), `fail` (the wiki says it is a problem, or the firmware logged the
failsafe), `not_applicable` (the log lacks the data, or the rule does not
cover this vehicle).

Rows whose timestamp is damaged (outside the log's time range, counted in
`integrity.time_outliers`; see [reading.md](reading.md#time)) are left out
of every check, event and armed range.

## Vehicles

The wiki pages the thresholds come from are the Copter pages
(`ardupilot.org/copter/docs/...`); several are common pages that every
vehicle's wiki publishes (vibration, log diagnosis, EKF failsafe). The checks
use `log.vehicle` (DataFlash `VER.BU` or the firmware string; tlog
`HEARTBEAT.type`) as follows:

- Copter and Heli logs get every rule, including the Copter wiki defaults
  (`FS_EKF_THRESH` 0.8, `BATT_LOW_VOLT` 10.5 V, `FS_GCS_TIMEOUT` 5 s,
  `RC_FS_TIMEOUT` 1 s) when the log has no value for the parameter.
- Other vehicles (Plane, Rover, Sub, Blimp, Tracker) never get a Copter
  default. A rule that needs a parameter uses the log's own value, which
  Plane, for example, logs for `FS_EKF_THRESH`, `BATT_LOW_VOLT` and
  `RC_FS_TIMEOUT`; without it the check is `not_applicable` and says why.
  `vibration.failsafe` is `not_applicable` for them: it is a Copter feature.
- When the vehicle is not identified, the defaults are not used either.

A parameter whose logged value is not a finite number (a damaged PARM row)
is treated as missing: `params_used` records it with `"value": null,
"origin": "param_invalid"`, and the check uses the Copter default if it may,
else none.

The test corpus has one Plane log (`plane-throttle-failsafe.bin`, ArduPlane
SITL `ThrottleFailsafe`); see [testing.md](testing.md).

"While armed" means inside the armed ranges (DataFlash `ARM.ArmState`, or
`EV` ids 10/11 on older logs; tlog: the vehicle's `HEARTBEAT.base_mode`
safety-armed bit). When a log has no arming information, the whole log is
used.

## Checks

Each rule below is exactly what the code does.

### vibration.levels

Data: `VIBE.VibeX/Y/Z` per `IMU` instance (tlog: `VIBRATION.vibration_x/y/z`),
while armed.

Wiki (common-measuring-vibration): "Vibration levels below 30m/s/s are
normally acceptable. Levels above 30m/s/s *may* have problems and levels above
60m/s/s nearly always have problems with position or altitude hold." and
"Check the recorded Vibe levels are mostly below 30m/s/s".

Rule: "mostly" is read as the 95th percentile (nearest rank, option
`vibe_percentile`). `fail` if any axis p95 > 60; `warn` if any axis p95 > 30;
`info` if p95 <= 30 but some sample > 30; else `pass`. Ranges: every stretch
above 30 and above 60 (stretches less than 1 s apart are joined).

### vibration.clipping

Data: `VIBE.Clip` per IMU (older logs: `Clip0..2`; tlog: `clipping_0..2`).
The counters are cumulative since boot; the total is the largest value.

Wiki: "Ideally these should be zero for the whole flight but low numbers
(<100) are likely ok especially if they occur during hard landings. A
regularly increasing number through the log indicates a serious vibration
problem".

Rule: `pass` at 0; `info` for 1 to 99; `warn` at 100 or more. Ranges: where
a counter rose.

### vibration.failsafe

Data: `ERR` subsystem 29 (Vibration Failsafe), code 1 to code 0. tlog:
`STATUSTEXT` "Vibration compensation ON" to "Vibration compensation OFF"
(the texts ArduCopter's `ekf_check.cpp` sends).

Wiki (vibration-failsafe): "The vibration failsafe will trigger if all of the
following are true for at least one second:" ...

Rule: `warn` if it was active, with its ranges; else `pass`.
`not_applicable` for vehicles other than Copter and Heli.

### ekf.variance

Data: `XKF4` (EKF3) or `NKF4` (EKF2) `SV`, `SP`, `SM` per core `C`, and
`PI` (the primary core index) when logged, while armed (tlog:
`EKF_STATUS_REPORT` velocity, horizontal position and compass variances,
which are the primary core's), and `ERR` subsystems 16 (EKF Check) and 17
(EKF Failsafe).

Wiki (common-ekf-inav-failsafe): "The EKF failsafe will trigger when any two
of the EKF "variances" for compass, position or velocity are higher than the
FS_EKF_THRESH parameter value for 1 second." The threshold is the log's
`FS_EKF_THRESH`; if missing or 0 (failsafe disabled), the 0.8 the wiki quotes
is used and marked `param_default` (Copter and Heli logs only, see
[Vehicles](#vehicles)).

Rule: `fail` if `ERR` 17 code 1 was logged (or the tlog text "EKF Failsafe"),
or two or more variances of the primary core stayed above the threshold
together for 1 s or longer (the failsafe reads the primary core: rows where
`C == PI`, or core 0 when `PI` is not logged); `warn` if a core that was not
the primary did, if one variance did (or `ERR` 16 code 2); else `pass`.
Without a threshold (another vehicle, no `FS_EKF_THRESH` in the log):
`not_applicable` unless the failsafe was logged, with the maximum variances
still in `measured`.

### gps.hdop and gps.satellites

Data: primary receiver (`GPS` instance 0) rows with `Status >= 3` (3D fix),
while armed. tlog: `GPS_RAW_INT` with `fix_type >= 3`, `eph / 100`
(65535 = unknown is skipped) and `satellites_visible`.

Wiki: "Hdop values below 1.5 are very good, values over 2.0 could indicate
the GPS positions are not good. The number of satellites falling below 12 is
also bad." (tlog: "Hdop values below 150 are good, values over 200 could
indicate a bad position.")

Rules: hdop `warn` if any sample > 2.0, `info` if the max is 1.5 to 2.0,
else `pass`. satellites `warn` if any sample < 12, else `pass`. Note: SITL
simulates 10 satellites, so every SITL log gets `gps.satellites: warn`. That
is what the wiki rule says, so the check says it.

### gps.glitch

Data: `ERR` subsystem 11 code 2 (GPS Glitch occurred) to code 0. tlog:
`STATUSTEXT` "GPS Glitch or Compass error" to "Glitch cleared".

Rule: `fail` for every logged glitch, with its range; else `pass`.

### battery.voltage

Data: `BAT` instance 0 `Volt` and `VoltR` while armed (tlog:
`SYS_STATUS.voltage_battery`), the log's `BATT_LOW_VOLT`, `BATT_CRT_VOLT`,
`BATT_LOW_TIMER` and `BATT_FS_VOLTSRC`, and `ERR` subsystem 6.

Wiki (failsafe-battery): the failsafe triggers when the "voltage drops below
the voltage held in the BATT_LOW_VOLT parameter ... for more than 10
seconds" and `BATT_FS_VOLTSRC` "allows configuring whether the raw battery
voltage or a sag corrected voltage is used".

Rule: the compared voltage is `Volt`, or `VoltR` when `BATT_FS_VOLTSRC = 1`.
`fail` if `ERR` 6 logged a battery failsafe, or the voltage stayed below
`BATT_LOW_VOLT` or `BATT_CRT_VOLT` for longer than `BATT_LOW_TIMER`; `warn`
instead of `fail` when `BATT_LOW_VOLT` is the Copter default because the log
has none (the vehicle's own threshold is unknown, for example on a 2S
battery); `warn` if it dipped below `BATT_LOW_VOLT` for less; `info` if the
voltage trigger is disabled (`BATT_LOW_VOLT = 0`); `not_applicable` when
there is no threshold (not Copter or Heli, no `BATT_LOW_VOLT` in the log) and
no failsafe was logged; else `pass`. Measured: minimum voltage, and
the largest sag (`VoltR - Volt`).

### power.board_vcc

Data: `POWR.Vcc` (tlog: `POWER_STATUS.Vcc`) while armed. SITL does not log
`POWR`, so SITL DataFlash logs report `not_applicable`.

Wiki: "Variations of 0.10 to 0.15 volts are normal. Beyond that could be a
sign that other devices sharing the power source with the autopilot are
causing ripples in the power supply that could lead to a brown-out."

Rule: `warn` if max - min > 0.15 V, else `pass`.

### power.log_ends_armed

Wiki: brown-outs "can normally be recognised by the logs suddenly ending
while the vehicle is still in the air".

Rule: DataFlash `warn` if the log ends while armed (the wiki adds: check the
altitude at the end); tlog `info` (a ground station may just stop
recording); else `pass`.

### rc.failsafe

Data: `ERR` subsystem 5 (Radio Failsafe) code 1 to 0, and subsystem 2
(Radio) code 2 "Late Frame : no updates received from receiver for two
seconds". Plane and Rover do not log `ERR` for it, so when a DataFlash log
has no such `ERR` the `MSG` texts are read, like the tlog `STATUSTEXT`:
"Radio Failsafe..." to "Radio Failsafe Cleared" (Copter, Rover), "Throttle
failsafe on" to "Throttle failsafe off" (Plane).

Rule: `fail` for a radio failsafe; `warn` for late frames without one; else
`pass`. `RC_FS_TIMEOUT` is reported.

### link.gcs_failsafe

Data: `ERR` subsystem 8 code 1 to 0; without it, the texts "GCS Failsafe"
to "GCS Failsafe Cleared" (DataFlash `MSG`, tlog `STATUSTEXT`). tlog also:
gaps in the vehicle's `HEARTBEAT` stream longer than `FS_GCS_TIMEOUT`
(default 5 s per the Copter wiki, for Copter and Heli logs) while armed.

Rule: `fail` for a logged GCS failsafe; tlog `warn` for heartbeat gaps;
else `pass`.

## Events

`events` lists, in time order: `ERR` rows decoded with the wiki's subsystem
and code table, `armed` / `disarmed` (no `disarmed` when the log ends armed),
flight mode changes (repeats of the same mode are dropped; names from
pymavlink's mode tables for Copter, Plane and Rover), and the failsafe text
lines (`MSG` in DataFlash logs, `STATUSTEXT` in tlogs).

## Versioning

`flight_facts_version` is `MAJOR.MINOR`. A minor version may add checks,
fields or event kinds; consumers must ignore what they do not know. Changing
the meaning of an existing check id or field is a major version.

- 1.1 (flightlog 0.1.1): `integrity.time_outliers`; `param_invalid` origin in
  `params_used`; `not_applicable` for rules the Copter wiki does not give the
  vehicle; the EKF failsafe rule on the primary core; `integrity.clean`
  documented as the same definition as the reader's. A 1.0 document is valid
  against the 1.1 schema.

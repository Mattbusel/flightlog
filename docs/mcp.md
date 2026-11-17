# flightlog-mcp

A read-only [Model Context Protocol](https://modelcontextprotocol.io) server
over ArduPilot logs. An assistant gets the flight-facts document (wiki
thresholds, measured values, time ranges) and can pull the raw rows behind
any finding, instead of parsing logs itself.

```text
pip install "flightlog[mcp]"
flightlog-mcp --root /path/to/logs [--root /other/dir] [--max-bytes N]
```

Transport is stdio. Only files under the `--root` directories can be read
(default: `FLIGHTLOG_MCP_ROOTS`, separated by the OS path separator, or the
current directory). Paths are resolved before the check, so `..` and links
cannot leave the roots. Nothing is written and nothing is sent anywhere; the
four most recently used logs stay in memory.

## Client configuration

Claude Code:

```text
claude mcp add flightlog -- flightlog-mcp --root /path/to/logs
```

Claude Desktop, Cursor and other clients that take a JSON server list:

```json
{
  "mcpServers": {
    "flightlog": {
      "command": "flightlog-mcp",
      "args": ["--root", "/path/to/logs"]
    }
  }
}
```

## Tools

All tools are annotated read-only, idempotent and closed-world.

| Tool | Arguments | Returns |
|---|---|---|
| `list_logs` | | `.bin` / `.tlog` files under the roots |
| `log_overview` | `path` | format, vehicle, firmware, duration, armed time, message counts, integrity, check summary |
| `message_fields` | `path`, `message` | columns, Arrow types, units, instance field |
| `query_messages` | `path`, `message`, `fields?`, `start_s?`, `end_s?`, `instance?`, `max_rows?` | rows in a time window, evenly thinned to at most `max_rows` (cap 2000), each with `t_s` |
| `field_stats` | `path`, `message`, `field`, `start_s?`, `end_s?`, `instance?` | count, min, max, mean, p05, p50, p95 |
| `flight_facts` | `path`, `check?` | the flight-facts v1 document, or one check |
| `get_params` | `path`, `prefix?` | parameter values |
| `get_events` | `path` | ERR (decoded), arming, mode changes, failsafe texts |

`start_s` / `end_s` are seconds from the first message of the log, the same
scale as `start_s` / `end_s` in flight-facts ranges, so a finding's range can
be passed straight to `query_messages`.

Relative paths are taken relative to the first root.

## Example session

On `copter-throttle-failsafe.bin` from the test corpus:

1. `list_logs`, then `log_overview(path)`.
2. `flight_facts(path, check="ekf.variance")` returns `fail`, "The EKF
   failsafe triggered 5 time(s).", with a first range
   `start_s: 588.082, end_s: 645.482` ("ERR EKF Failsafe: Failsafe Triggered").
3. `query_messages(path, "XKF4", fields=["SV", "SP", "SM"], start_s=580, end_s=600)`
   and `get_events(path)` show what led up to it (in this autotest, the radio
   failsafe that `rc.failsafe` reports starting at 586.707 s).

"""flightlog-mcp: a read-only MCP server for ArduPilot flight logs.

    pip install "flightlog[mcp]"
    flightlog-mcp --root /path/to/logs            # stdio transport

Every tool takes a ``path`` to a ``.bin`` or ``.tlog`` file inside one of
the ``--root`` directories (default: the current directory; also
``FLIGHTLOG_MCP_ROOTS``, separated by the OS path separator). Nothing is
written, nothing leaves the machine. Tools:

* ``list_logs``      .bin and .tlog files under the roots
* ``log_overview``   format, vehicle, firmware, duration, message counts, integrity
* ``message_fields`` columns, types and units of one message type
* ``query_messages`` rows of one message type, time-windowed and evenly thinned
* ``field_stats``    min / max / mean / percentiles of one field
* ``flight_facts``   the flight-facts v1 document (or one check)
* ``get_params``     parameters, optionally by prefix
* ``get_events``     ERR, arming, mode changes and failsafe texts
"""

from __future__ import annotations

import argparse
import math
import os
import pathlib
import threading
from collections import OrderedDict
from typing import Any, Dict, List, Optional

import pyarrow as pa
import pyarrow.compute as pc

import flightlog

MAX_ROWS = 2000
CACHE_SIZE = 4


class LogStore:
    """Opens logs inside the allowed roots and keeps a few in memory."""

    def __init__(self, roots: List[str], max_bytes: int = 4 << 30):
        self.roots = [pathlib.Path(r).resolve() for r in roots]
        self.max_bytes = max_bytes
        self._cache: "OrderedDict[tuple, flightlog.Log]" = OrderedDict()
        self._lock = threading.Lock()

    def resolve(self, path: str) -> pathlib.Path:
        p = pathlib.Path(path)
        if not p.is_absolute() and self.roots:
            p = self.roots[0] / p
        p = p.resolve()
        if not any(p == r or r in p.parents for r in self.roots):
            raise ValueError(f"{path} is outside the allowed directories: {', '.join(map(str, self.roots))}")
        if not p.is_file():
            raise ValueError(f"{path} is not a file")
        if p.stat().st_size > self.max_bytes:
            raise ValueError(f"{path} is larger than the {self.max_bytes} byte limit")
        return p

    def open(self, path: str) -> flightlog.Log:
        p = self.resolve(path)
        st = p.stat()
        key = (str(p), st.st_size, st.st_mtime_ns)
        with self._lock:
            if key in self._cache:
                self._cache.move_to_end(key)
                return self._cache[key]
        log = flightlog.open(p)
        with self._lock:
            self._cache[key] = log
            while len(self._cache) > CACHE_SIZE:
                self._cache.popitem(last=False)
        return log

    def list_logs(self, limit: int = 200) -> List[Dict[str, Any]]:
        out = []
        for r in self.roots:
            for p in sorted(r.rglob("*")):
                if p.suffix.lower() in (".bin", ".tlog") and p.is_file():
                    out.append({"path": str(p), "bytes": p.stat().st_size})
                    if len(out) >= limit:
                        return out
        return out


def _clean(v: Any) -> Any:
    if isinstance(v, float):
        return v if math.isfinite(v) else None
    if isinstance(v, bytes):
        return v.hex()
    if isinstance(v, list):
        return [_clean(x) for x in v]
    if isinstance(v, dict):
        return {k: _clean(x) for k, x in v.items()}
    return v


def _window(log: flightlog.Log, t: pa.Table, start_s: Optional[float], end_s: Optional[float]) -> pa.Table:
    rng = log.time_range_us
    tc = log.time_column
    if rng is None or tc not in t.column_names or (start_s is None and end_s is None):
        return t
    col = pc.cast(t.column(tc), pa.float64())
    mask = None
    if start_s is not None:
        mask = pc.greater_equal(col, rng[0] + start_s * 1e6)
    if end_s is not None:
        m2 = pc.less_equal(col, rng[0] + end_s * 1e6)
        mask = m2 if mask is None else pc.and_(mask, m2)
    return t.filter(mask)


def log_overview(store: LogStore, path: str) -> Dict[str, Any]:
    log = store.open(path)
    facts = log.facts(events=False)
    return {
        "path": str(store.resolve(path)),
        "log": facts["log"],
        "armed": {"source": facts["armed"]["source"], "total_s": facts["armed"]["total_s"]},
        "integrity": facts["integrity"],
        "message_counts": log.counts,
        "check_summary": facts["summary"],
    }


def message_fields(store: LogStore, path: str, message: str) -> Dict[str, Any]:
    log = store.open(path)
    return {
        "message": message,
        "rows": log.counts.get(message, 0),
        "instance_field": log.instance_field(message),
        "fields": [{"name": n, "type": t, "unit": u} for n, t, u in log.columns(message)],
    }


def query_messages(store: LogStore, path: str, message: str, fields: Optional[List[str]] = None,
                   start_s: Optional[float] = None, end_s: Optional[float] = None,
                   instance: Optional[int] = None, max_rows: int = 200) -> Dict[str, Any]:
    log = store.open(path)
    max_rows = max(1, min(int(max_rows), MAX_ROWS))
    t = _window(log, log.table(message, instance=instance), start_s, end_s)
    if fields:
        keep = [c for c in [log.time_column] + list(fields) if c in t.column_names]
        missing = [f for f in fields if f not in t.column_names]
        if missing:
            raise ValueError(f"{message} has no field(s) {missing}; fields: {t.column_names}")
        t = t.select(list(dict.fromkeys(keep)))
    total = t.num_rows
    step = max(1, math.ceil(total / max_rows))
    if step > 1:
        t = t.take(pa.array(range(0, total, step), pa.int64()))
    rng = log.time_range_us
    rows = t.to_pylist()
    if rng is not None and log.time_column in t.column_names:
        for r in rows:
            r["t_s"] = round((r[log.time_column] - rng[0]) / 1e6, 6)
    return {"message": message, "rows_in_window": total, "every_nth": step, "returned": len(rows), "rows": _clean(rows)}


def field_stats(store: LogStore, path: str, message: str, field: str, start_s: Optional[float] = None,
                end_s: Optional[float] = None, instance: Optional[int] = None) -> Dict[str, Any]:
    log = store.open(path)
    t = _window(log, log.table(message, instance=instance), start_s, end_s)
    if field not in t.column_names:
        raise ValueError(f"{message} has no field {field}; fields: {t.column_names}")
    col = t.column(field)
    if not (pa.types.is_integer(col.type) or pa.types.is_floating(col.type)):
        raise ValueError(f"{message}.{field} is not numeric ({col.type})")
    col = pc.cast(col, pa.float64())
    finite = col.filter(pc.is_finite(col)) if len(col) else col
    out: Dict[str, Any] = {"message": message, "field": field, "count": len(col), "finite": len(finite)}
    if len(finite):
        q = pc.quantile(finite, q=[0.05, 0.5, 0.95]).to_pylist()
        out.update({
            "min": pc.min(finite).as_py(), "max": pc.max(finite).as_py(), "mean": pc.mean(finite).as_py(),
            "p05": q[0], "p50": q[1], "p95": q[2],
        })
    return _clean(out)


def flight_facts(store: LogStore, path: str, check: Optional[str] = None) -> Dict[str, Any]:
    doc = store.open(path).facts()
    if check is None:
        return doc
    for c in doc["checks"]:
        if c["id"] == check:
            return c
    raise ValueError(f"no check {check}; checks: {[c['id'] for c in doc['checks']]}")


def get_params(store: LogStore, path: str, prefix: Optional[str] = None) -> Dict[str, Any]:
    params = store.open(path).params
    if prefix:
        params = {k: v for k, v in params.items() if k.startswith(prefix.upper())}
    return _clean(params)


def get_events(store: LogStore, path: str) -> List[Dict[str, Any]]:
    return store.open(path).facts()["events"]


def build_server(store: LogStore):
    try:
        from mcp.server.mcpserver import MCPServer as Server  # mcp >= 2
    except ImportError:  # pragma: no cover - mcp 1.x
        from mcp.server.fastmcp import FastMCP as Server
    from mcp.types import ToolAnnotations

    server = Server("flightlog", instructions=(
        "Read-only access to ArduPilot flight logs (.bin DataFlash and .tlog). "
        "Start with list_logs or log_overview, then flight_facts for wiki-sourced checks; "
        "query_messages and field_stats give the raw data behind any finding. "
        "Times: start_s/end_s are seconds from the first message of the log."))
    ro = ToolAnnotations(readOnlyHint=True, destructiveHint=False, idempotentHint=True, openWorldHint=False)

    @server.tool(name="list_logs", annotations=ro)
    def _list_logs() -> List[Dict[str, Any]]:
        """Lists .bin and .tlog files under the allowed directories."""
        return store.list_logs()

    @server.tool(name="log_overview", annotations=ro)
    def _overview(path: str) -> Dict[str, Any]:
        """Format, vehicle, firmware, duration, armed time, message counts, reader integrity and check summary of a log."""
        return log_overview(store, path)

    @server.tool(name="message_fields", annotations=ro)
    def _fields(path: str, message: str) -> Dict[str, Any]:
        """Columns, Arrow types and units of one message type (e.g. ATT, GPS, VIBE, XKF4 or GPS_RAW_INT)."""
        return message_fields(store, path, message)

    @server.tool(name="query_messages", annotations=ro)
    def _query(path: str, message: str, fields: Optional[List[str]] = None, start_s: Optional[float] = None,
              end_s: Optional[float] = None, instance: Optional[int] = None, max_rows: int = 200) -> Dict[str, Any]:
        """Rows of one message type between start_s and end_s (seconds from log start), evenly thinned to max_rows (at most 2000)."""
        return query_messages(store, path, message, fields, start_s, end_s, instance, max_rows)

    @server.tool(name="field_stats", annotations=ro)
    def _stats(path: str, message: str, field: str, start_s: Optional[float] = None, end_s: Optional[float] = None,
              instance: Optional[int] = None) -> Dict[str, Any]:
        """min, max, mean, p05, p50 and p95 of one numeric field, optionally in a time window."""
        return field_stats(store, path, message, field, start_s, end_s, instance)

    @server.tool(name="flight_facts", annotations=ro)
    def _facts(path: str, check: Optional[str] = None) -> Dict[str, Any]:
        """flight-facts v1: deterministic checks (vibration, clipping, EKF variance, GPS, battery, board voltage, RC and GCS failsafes), each with its ArduPilot wiki threshold, quote, measured values and time ranges. Pass check= for one check."""
        return flight_facts(store, path, check)

    @server.tool(name="get_params", annotations=ro)
    def _params(path: str, prefix: Optional[str] = None) -> Dict[str, Any]:
        """Parameter values in the log, optionally only names starting with prefix (e.g. BATT_)."""
        return get_params(store, path, prefix)

    @server.tool(name="get_events", annotations=ro)
    def _events(path: str) -> List[Dict[str, Any]]:
        """ERR events (decoded with the wiki's table), arming, mode changes and failsafe texts, in time order."""
        return get_events(store, path)

    return server


def main(argv: Optional[List[str]] = None) -> None:
    ap = argparse.ArgumentParser(prog="flightlog-mcp", description="Read-only MCP server for ArduPilot flight logs.")
    ap.add_argument("--root", action="append", default=None,
                    help="directory logs may be read from (repeatable; default: FLIGHTLOG_MCP_ROOTS or the current directory)")
    ap.add_argument("--max-bytes", type=int, default=4 << 30, help="largest log file to open")
    args = ap.parse_args(argv)
    roots = args.root
    if not roots:
        env = os.environ.get("FLIGHTLOG_MCP_ROOTS")
        roots = env.split(os.pathsep) if env else [os.getcwd()]
    build_server(LogStore(roots, args.max_bytes)).run()


if __name__ == "__main__":
    main()

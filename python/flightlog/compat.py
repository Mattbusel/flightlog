"""pymavlink-style message-by-message reading on top of flightlog.

``DFReader_binary`` mirrors the parts of ``pymavlink.DFReader.DFReader_binary``
that log tools use: ``recv_msg()``, ``recv_match(type=..., condition=...)``,
``rewind()``, ``params``, ``param()``, ``messages`` and message objects with
attribute access, ``get_type()``, ``to_dict()``, ``get_fieldnames()``,
``_timestamp``, ``fmt`` and ``_elements`` (the unscaled values, as pymavlink
stores them).

It reads the whole log with flightlog first, so a damaged file that makes
pymavlink raise (for example a corrupted FMT record) still opens; see
``integrity`` for what was skipped.

    from flightlog.compat import DFReader_binary

    log = DFReader_binary("00000001.BIN")
    while (m := log.recv_match(type=["ATT", "GPS"])) is not None:
        print(m.get_type(), m._timestamp, m.to_dict())
"""

from __future__ import annotations

import math
from typing import Any, Dict, Iterable, List, Optional, Union

from . import Log

__all__ = ["DFReader_binary", "DFMessage", "DFFormat"]

_SCALE = {"c": 100.0, "C": 100.0, "e": 100.0, "E": 100.0, "L": 1e7}
_TEXT_LEN = {"n": 4, "N": 16, "Z": 64}


class DFFormat:
    """Format of one message type (subset of pymavlink's ``DFFormat``)."""

    __slots__ = ("type", "name", "len", "format", "columns", "colhash", "instance_field", "units")

    def __init__(self, type_id: int, name: str, length: int, fmt: str, columns: List[str],
                 instance_field: Optional[str], units: List[Optional[str]]):
        self.type = type_id
        self.name = name
        self.len = length
        self.format = fmt
        self.columns = columns
        self.colhash = {c: i for i, c in enumerate(columns)}
        self.instance_field = instance_field
        self.units = units

    def get_unit(self, col: str) -> str:
        return self.units[self.colhash[col]] or ""

    def __str__(self) -> str:
        return "DFFormat(%s,%s,%s,%s)" % (self.type, self.name, self.format, self.columns)


class DFMessage:
    """One message. Attributes are the fields, values as pymavlink returns them."""

    __slots__ = ("fmt", "_values", "_timestamp", "_fieldnames")

    def __init__(self, fmt: DFFormat, values: List[Any], timestamp: float):
        self.fmt = fmt
        self._values = values
        self._fieldnames = fmt.columns
        self._timestamp = timestamp

    def __getattr__(self, field: str) -> Any:
        try:
            return self._values[self.fmt.colhash[field]]
        except KeyError:
            raise AttributeError(field) from None

    def __getitem__(self, key: str) -> Any:
        return self.__getattr__(key)

    def get_type(self) -> str:
        return self.fmt.name

    def get_fieldnames(self) -> List[str]:
        return list(self._fieldnames)

    def to_dict(self) -> Dict[str, Any]:
        d: Dict[str, Any] = {"mavpackettype": self.fmt.name}
        for i, f in enumerate(self._fieldnames):
            d[f] = self._values[i]
        return d

    @property
    def _elements(self) -> List[Any]:
        """Unscaled values as pymavlink's struct unpacking gives them."""
        out = []
        for c, v in zip(self.fmt.format, self._values):
            if c in _SCALE and isinstance(v, float) and math.isfinite(v):
                out.append(int(round(v * _SCALE[c])))
            elif c in _TEXT_LEN and isinstance(v, str):
                out.append(v.encode("utf-8").ljust(_TEXT_LEN[c], b"\0")[: _TEXT_LEN[c]])
            else:
                out.append(v)
        return out

    def __str__(self) -> str:
        return "%s {%s}" % (self.fmt.name, ", ".join("%s : %s" % (f, v) for f, v in zip(self._fieldnames, self._values)))

    __repr__ = __str__


class DFReader_binary:  # noqa: N801  (pymavlink's name)
    """Reads a DataFlash log message by message, like pymavlink's reader."""

    def __init__(self, filename: str, zero_time_base: bool = False, progress_callback=None):
        self.filename = filename
        self._log = Log(filename, keep_order=True)
        if self._log.kind != "dataflash":
            raise ValueError("%s is not a DataFlash log" % filename)
        self.integrity = self._log.integrity
        # pymavlink fills params with every PARM while working out the clock
        self.params: Dict[str, float] = dict(self._log.params)
        self.messages: Dict[str, Any] = {}
        self._names = self._log.message_types
        self._formats: List[DFFormat] = []
        self._rows: List[Optional[List[List[Any]]]] = [None] * len(self._names)
        fmt_rows = self._fmt_lookup()
        for i, name in enumerate(self._names):
            cols = [c[0] for c in self._log.columns(name)]
            units = [c[2] for c in self._log.columns(name)]
            type_id, length, fmt = fmt_rows.get(name.split("#")[0], (0, 0, ""))
            self._formats.append(DFFormat(type_id, name, length, fmt, cols, self._log.instance_field(name), units))
        order = self._log.order()
        self._order_type = order.column("type").to_pylist()
        self._order_row = order.column("row").to_pylist()
        self._time_cols = [f.colhash.get("TimeUS") for f in self._formats]
        base = 0.0 if zero_time_base or self._log.utc_offset_us is None else self._log.utc_offset_us * 1e-6
        self._timebase = base
        self._pos = 0
        self._timestamp = base
        self._prefill()
        if progress_callback is not None:
            progress_callback(100)

    def _first_rows(self):
        """pymavlink indexes the file on open and keeps the first message of
        each type (and of each instance) in ``messages``; GPS is the one its
        clock setup stopped at (first with a week number)."""
        out = {}
        for i, name in enumerate(self._names):
            fmt = self._formats[i]
            t = self._log.table(name)
            if t.num_rows == 0:
                continue
            picks = {name: 0}
            if fmt.instance_field is not None:
                seen = {}
                for r, v in enumerate(t.column(fmt.instance_field).to_pylist()):
                    if v not in seen:
                        seen[v] = r
                for v, r in seen.items():
                    picks["%s[%s]" % (name, v)] = r
            if name == "GPS" and "GWk" in fmt.colhash:
                wk = t.column("GWk").to_pylist()
                first = next((r for r, w in enumerate(wk) if w and w > 0), None)
                if first is not None:
                    picks[name] = first
            for key, r in picks.items():
                row = [c[0] for c in t.slice(r, 1).to_pydict().values()]
                ts = self._timebase + row[0] * 1e-6 if fmt.columns and fmt.columns[0] == "TimeUS" else self._timebase
                out[key] = DFMessage(fmt, row, ts)
        return out

    def _prefill(self):
        self._initial_messages = self._first_rows()
        self.messages.update(self._initial_messages)

    def _fmt_lookup(self):
        out = {}
        if "FMT" not in self._log:
            return out
        t = self._log.table("FMT")
        for ty, ln, nm, fm in zip(t.column("Type").to_pylist(), t.column("Length").to_pylist(),
                                  t.column("Name").to_pylist(), t.column("Format").to_pylist()):
            out[nm] = (ty, ln, fm)
        return out

    def _table_rows(self, i: int) -> List[List[Any]]:
        rows = self._rows[i]
        if rows is None:
            t = self._log.table(self._names[i])
            cols = [c.to_pylist() for c in t.columns]
            rows = [list(r) for r in zip(*cols)] if cols else [[] for _ in range(t.num_rows)]
            self._rows[i] = rows
        return rows

    def rewind(self) -> None:
        """Back to the first message."""
        self._pos = 0
        self._timestamp = self._timebase
        self.messages.clear()
        self.messages.update(self._initial_messages)

    _rewind = rewind

    def recv_msg(self) -> Optional[DFMessage]:
        """The next message, or None at the end."""
        if self._pos >= len(self._order_type):
            return None
        ti = self._order_type[self._pos]
        row = self._table_rows(ti)[self._order_row[self._pos]]
        self._pos += 1
        fmt = self._formats[ti]
        tc = self._time_cols[ti]
        if tc is not None and fmt.columns and fmt.columns[0] == "TimeUS":
            self._timestamp = self._timebase + row[tc] * 1e-6
        m = DFMessage(fmt, row, self._timestamp)
        name = fmt.name
        self.messages[name] = m
        if fmt.instance_field is not None:
            self.messages["%s[%s]" % (name, getattr(m, fmt.instance_field))] = m
        if name == "PARM":
            self.params[m.Name] = m.Value
        return m

    def recv_match(self, condition: Optional[str] = None, type: Union[None, str, Iterable[str]] = None,  # noqa: A002
                   blocking: bool = False, strict: bool = False) -> Optional[DFMessage]:
        """The next message of the given type(s) for which ``condition`` holds."""
        types = None
        if type is not None:
            types = {type} if isinstance(type, str) else set(type)
        while True:
            m = self.recv_msg()
            if m is None:
                return None
            if types is not None and m.get_type() not in types:
                continue
            if condition is not None and not self._check(condition):
                continue
            return m

    def _check(self, condition: str) -> bool:
        # Same rule as pymavlink.mavutil.evaluate_condition: a Python
        # expression over the latest message of each type.
        try:
            return bool(eval(condition, {"__builtins__": {}, "math": math}, dict(self.messages)))  # noqa: S307
        except Exception:
            return False

    def check_condition(self, condition: str) -> bool:
        return self._check(condition)

    def param(self, name: str, default: Any = None) -> Any:
        """Parameter value seen so far, like pymavlink (all of them via ``log_params``)."""
        return self.params.get(name, default)

    @property
    def log_params(self) -> Dict[str, float]:
        """Final value of every parameter in the log."""
        return self._log.params

    def close(self) -> None:
        self._rows = [None] * len(self._names)

    def __enter__(self) -> "DFReader_binary":
        return self

    def __exit__(self, *exc) -> None:
        self.close()

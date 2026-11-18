"""ArduPilot flight logs as Arrow tables.

    import flightlog

    log = flightlog.open("00000001.BIN")      # or a .tlog
    att = log.table("ATT")                    # pyarrow.Table
    imu1 = log.table("IMU", instance=1)       # one instance
    df = log.polars("GPS")                    # polars.DataFrame
    facts = log.facts()                       # flight-facts v1 (dict)

DataFlash values match pymavlink's DFReader (scaled ``c/C/e/E/L`` fields,
text cut at NUL); tlog fields follow the MAVLink ``all.xml`` dialect.
``flightlog.compat.DFReader_binary`` offers pymavlink's message-by-message
interface on top of the same reader.
"""

from __future__ import annotations

import json
import os
import re
from typing import Any, Dict, Iterable, Optional

from ._flightlog import FACTS_VERSION, __version__, detect, facts_schema
from ._flightlog import Log as _Log

__all__ = ["Log", "open", "from_bytes", "read_table", "detect", "facts_schema", "parquet_file_name", "FACTS_VERSION", "__version__"]


class Log(_Log):
    """A flight log read into memory: one Arrow table per message type.

    ``Log(source, types=None, scale=True, keep_order=False)``; ``source`` is
    a path or the log's bytes.

    * ``types``: only build tables for these message names (others are still
      counted in ``counts``); parameters are always read.
    * ``scale``: apply DataFlash format multipliers like pymavlink (default).
    * ``keep_order``: remember the file order of all messages (``order()``).
    """

    def tables(self, names: Optional[Iterable[str]] = None) -> Dict[str, Any]:
        """``{name: pyarrow.Table}`` for the given names (default: all)."""
        return {n: self.table(n) for n in (names if names is not None else self.message_types)}

    def polars(self, name: str, instance: Optional[int] = None):
        """One message type as a ``polars.DataFrame`` (needs ``polars``)."""
        import polars as pl

        return pl.from_arrow(self.table(name, instance=instance))

    def pandas(self, name: str, instance: Optional[int] = None):
        """One message type as a ``pandas.DataFrame`` (needs ``pandas``)."""
        return self.table(name, instance=instance).to_pandas()

    def facts(self, vibe_percentile: float = 0.95, events: bool = True) -> Dict[str, Any]:
        """flight-facts v1 for this log, as a dict (see ``facts_schema()``)."""
        return json.loads(self.facts_json(vibe_percentile=vibe_percentile, events=events))

    def to_parquet(self, directory: str, names: Optional[Iterable[str]] = None) -> Dict[str, str]:
        """Writes one Parquet file per message type; returns ``{name: path}``.

        Message names come from the log itself, so each becomes a safe file
        name (see ``parquet_file_name``) and every file stays inside
        ``directory``.
        """
        import pyarrow.parquet as pq

        os.makedirs(directory, exist_ok=True)
        root = os.path.realpath(directory)
        out = {}
        used = set()
        for n in names if names is not None else self.message_types:
            fname = parquet_file_name(n)
            if fname.lower() in used:
                raise ValueError(f"two message names map to the file name {fname}")
            used.add(fname.lower())
            p = os.path.join(root, fname)
            if os.path.dirname(os.path.realpath(p)) != root:
                raise ValueError(f"message name {n!r} does not map to a file inside {directory}")
            pq.write_table(self.table(n), p)
            out[n] = p
        return out


_SAFE = re.compile(r"[A-Za-z0-9_#-]")


def parquet_file_name(name: str) -> str:
    """The Parquet file name ``to_parquet`` uses for a message name.

    Letters, digits, ``_``, ``#`` and ``-`` are kept; every other character
    (including ``.``, ``/``, ``\\`` and ``:``) becomes ``%XX`` (its UTF-8
    bytes in hex), so a name from a crafted log cannot leave the directory.
    """
    safe = "".join(c if _SAFE.fullmatch(c) else _hex(c) for c in name)
    if safe.upper() in _RESERVED:
        # Windows device names (CON, NUL, COM1, ...) are not usable as files
        safe = _hex(safe[0]) + safe[1:]
    return f"{safe or '%'}.parquet"


def _hex(c: str) -> str:
    return "".join(f"%{b:02X}" for b in c.encode())


_RESERVED = {"CON", "PRN", "AUX", "NUL"} | {f"{p}{i}" for p in ("COM", "LPT") for i in range(10)}


def from_bytes(data, types: Optional[Iterable[str]] = None, scale: bool = True, keep_order: bool = False) -> Log:
    """Reads a log held in memory (``bytes``, ``bytearray``, ``memoryview``)."""
    return Log(data, types=list(types) if types is not None else None, scale=scale, keep_order=keep_order)


def open(path, types: Optional[Iterable[str]] = None, scale: bool = True, keep_order: bool = False) -> Log:  # noqa: A001
    """Reads a DataFlash ``.bin`` or MAVLink ``.tlog`` file."""
    return Log(os.fspath(path), types=list(types) if types is not None else None, scale=scale, keep_order=keep_order)


def read_table(path, name: str, instance: Optional[int] = None, scale: bool = True):
    """Reads one message type of a log as a ``pyarrow.Table``."""
    return Log(os.fspath(path), types=[name], scale=scale).table(name, instance=instance)

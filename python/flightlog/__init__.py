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
from typing import Any, Dict, Iterable, Optional

from ._flightlog import FACTS_VERSION, __version__, detect, facts_schema
from ._flightlog import Log as _Log

__all__ = ["Log", "open", "from_bytes", "read_table", "detect", "facts_schema", "FACTS_VERSION", "__version__"]


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
        """Writes one Parquet file per message type; returns ``{name: path}``."""
        import pyarrow.parquet as pq

        os.makedirs(directory, exist_ok=True)
        out = {}
        for n in names if names is not None else self.message_types:
            p = os.path.join(directory, f"{n}.parquet")
            pq.write_table(self.table(n), p)
            out[n] = p
        return out


def from_bytes(data, types: Optional[Iterable[str]] = None, scale: bool = True, keep_order: bool = False) -> Log:
    """Reads a log held in memory (``bytes``, ``bytearray``, ``memoryview``)."""
    return Log(data, types=list(types) if types is not None else None, scale=scale, keep_order=keep_order)


def open(path, types: Optional[Iterable[str]] = None, scale: bool = True, keep_order: bool = False) -> Log:  # noqa: A001
    """Reads a DataFlash ``.bin`` or MAVLink ``.tlog`` file."""
    return Log(os.fspath(path), types=list(types) if types is not None else None, scale=scale, keep_order=keep_order)


def read_table(path, name: str, instance: Optional[int] = None, scale: bool = True):
    """Reads one message type of a log as a ``pyarrow.Table``."""
    return Log(os.fspath(path), types=[name], scale=scale).table(name, instance=instance)

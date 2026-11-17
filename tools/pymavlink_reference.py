"""Reference summary of a log as pymavlink reads it.

    pip install pymavlink==2.4.50
    python tools/pymavlink_reference.py LOG > LOG.pymavlink.json

For every message type: the message count, the field names, and per field a
fingerprint of all values in file order:
  numbers  -> repr() of the float sum (ints are summed exactly, as a string)
  text     -> crc32 of the UTF-8 values each followed by a NUL byte
  arrays   -> sum over all elements
  bytes    -> crc32 of the values each followed by a NUL byte
plus the first row. tests/reference.rs computes the same from flightlog-core.

DataFlash logs are read with DFReader_binary.recv_msg() (the sequential
parser, including its recovery from damaged bytes). tlogs with
mavutil.mavlink_connection(dialect="all", robust_parsing=True) and one
recv_match(type=...) pass per type; BAD_DATA and UNKNOWN_* are left out. For tlogs the fingerprint also covers sys_id and
comp_id, and timestamp_us is checked against pymavlink's _timestamp.
"""
import array
import json
import math
import sys
import zlib

import pymavlink
from pymavlink import mavutil
from pymavlink.DFReader import DFReader_binary


def kind(path):
    with open(path, "rb") as f:
        head = f.read(9)
    if head[:2] == b"\xa3\x95":
        return "dataflash"
    return "tlog"


class Acc:
    def __init__(self):
        self.kind = None
        self.isum = 0
        self.fsum = 0.0
        self.crc = 0
        self.mixed = False

    def add(self, v):
        if isinstance(v, (array.array, list, tuple)):
            for x in v:
                self.add_num(x)
            return
        if isinstance(v, bytes):
            self.set("bytes")
            self.crc = zlib.crc32(v + b"\0", self.crc)
            return
        if isinstance(v, str):
            self.set("text")
            self.crc = zlib.crc32(v.encode("utf-8", "surrogateescape") + b"\0", self.crc)
            return
        self.add_num(v)

    def add_num(self, x):
        if isinstance(x, bool):
            x = int(x)
        if isinstance(x, int):
            if self.kind == "float":
                self.fsum += float(x)
            else:
                self.set("int")
                self.isum += x
        else:
            if self.kind == "int":
                # promote
                self.fsum = float(self.isum)
            self.kind = "float"
            self.fsum += x

    def set(self, k):
        if self.kind is None:
            self.kind = k
        elif self.kind != k:
            self.mixed = True

    def out(self):
        if self.kind == "int":
            return {"int": str(self.isum)}
        if self.kind == "float":
            return {"float": repr(self.fsum)}
        if self.kind in ("text", "bytes"):
            return {self.kind: self.crc}
        return {}


def first_row(d):
    out = {}
    for k, v in d.items():
        if isinstance(v, (array.array, list, tuple)):
            v = [repr(x) if isinstance(x, float) and not math.isfinite(x) else x for x in v]
        elif isinstance(v, bytes):
            v = v.hex()
        elif isinstance(v, float) and not math.isfinite(v):
            v = repr(v)
        out[k] = v
    return out


def main():
    path = sys.argv[1]
    k = kind(path)
    types = {}
    order = []
    if k == "dataflash":
        log = DFReader_binary(path, zero_time_base=True)
        log._rewind()
        log.offset = 0
        log.remaining = log.data_len
        while True:
            m = log.recv_msg()
            if m is None:
                break
            name = m.get_type()
            d = {f: getattr(m, f) for f in m._fieldnames}
            add(types, order, name, d)
    else:
        # pymavlink's sequential recv_msg() drops some valid frames that sit
        # next to non-MAVLink bytes, while recv_match(type=...) finds them;
        # use one type-filtered pass per type (names from a first pass).
        names = []
        conn = mavutil.mavlink_connection(path, dialect="all", robust_parsing=True)
        while True:
            m = conn.recv_msg()
            if m is None:
                break
            name = m.get_type()
            if name == "BAD_DATA" or name.startswith("UNKNOWN_") or name in names:
                continue
            names.append(name)
        for name in names:
            conn = mavutil.mavlink_connection(path, dialect="all", robust_parsing=True)
            while True:
                m = conn.recv_match(type=name)
                if m is None:
                    break
                d = {"timestamp_us": int(round(m._timestamp * 1e6)),
                     "sys_id": m.get_srcSystem(), "comp_id": m.get_srcComponent()}
                for f in m.get_fieldnames():
                    d[f] = getattr(m, f)
                add(types, order, name, d)
    result = {
        "pymavlink": pymavlink.__version__,
        "kind": k,
        "types": {n: types[n] for n in order},
    }
    for t in result["types"].values():
        t["fields"] = {f: a.out() for f, a in t["fields"].items()}
    # bytes, so the file has LF line endings on every OS
    sys.stdout.buffer.write((json.dumps(result, indent=1) + "\n").encode("utf-8"))


def add(types, order, name, d):
    t = types.get(name)
    if t is None:
        t = types[name] = {"count": 0, "first": first_row(d), "fields": {f: Acc() for f in d}}
        order.append(name)
    t["count"] += 1
    for f, v in d.items():
        acc = t["fields"].get(f)
        if acc is None:
            continue
        if f == "timestamp_us":
            acc.add(0)
            continue
        acc.add(v)


if __name__ == "__main__":
    main()

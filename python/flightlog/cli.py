"""flightlog command line.

    flightlog info LOG              message types, counts, integrity
    flightlog facts LOG [--pretty]  flight-facts v1 JSON
    flightlog export LOG DIR        one Parquet file per message type
    flightlog schema                the flight-facts v1 JSON Schema
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import List, Optional

import flightlog


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(prog="flightlog", description="ArduPilot flight logs as tables and facts.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("info", help="message types, counts and integrity")
    p.add_argument("log")
    p = sub.add_parser("facts", help="flight-facts v1 JSON")
    p.add_argument("log")
    p.add_argument("--pretty", action="store_true")
    p = sub.add_parser("export", help="one Parquet file per message type")
    p.add_argument("log")
    p.add_argument("dir")
    p.add_argument("--types", nargs="*")
    sub.add_parser("schema", help="the flight-facts v1 JSON Schema")
    a = ap.parse_args(argv)

    if a.cmd == "schema":
        print(flightlog.facts_schema())
        return 0
    log = flightlog.open(a.log, types=getattr(a, "types", None) or None)
    if a.cmd == "info":
        print(f"{a.log}: {log.kind}, {len(log)} messages, {len(log.counts)} types")
        for name, n in sorted(log.counts.items()):
            print(f"  {name:<20} {n:>9}")
        print(json.dumps(log.integrity, indent=1))
    elif a.cmd == "facts":
        doc = log.facts()
        print(json.dumps(doc, indent=2 if a.pretty else None))
        return 1 if doc["summary"]["fail"] else 0
    elif a.cmd == "export":
        for name, path in log.to_parquet(a.dir).items():
            print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())

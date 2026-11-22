#!/usr/bin/env bash
# Downloads the ArduPilot SITL autotest logs used by the reference and facts
# tests into testdata/external, checks their sha256, and makes the damaged
# copies (tools/make_damaged.py), and the PX4 ULog files from pyulog's test
# suite into testdata/external/ulog. See docs/testing.md for where they came from.
set -euo pipefail
cd "$(dirname "$0")/.."
BASE="${FLIGHTLOG_TESTDATA_URL:-https://github.com/Mattbusel/flightlog/releases/download/testdata-v1}"
mkdir -p testdata/external
cd testdata/external
while read -r sum name; do
  if [ ! -f "$name" ] || ! echo "$sum  $name" | sha256sum -c --status 2>/dev/null; then
    curl -fsSL --retry 3 -o "$name" "$BASE/$name"
  fi
done < ../SHA256SUMS
sha256sum -c ../SHA256SUMS
cd ../..
PY="${PYTHON:-python3}"
command -v "$PY" >/dev/null || PY=python
"$PY" tools/make_damaged.py testdata/external/copter-mission.bin

# PX4 ULog: pyulog's test files (BSD-3-Clause), pinned by commit and sha256
PYULOG="${PYULOG_COMMIT:-0b392f8444f843b796405b9766cc15a6dddedf62}"
mkdir -p testdata/external/ulog
cd testdata/external/ulog
while read -r sum name; do
  if [ ! -f "$name" ] || ! echo "$sum  $name" | sha256sum -c --status 2>/dev/null; then
    curl -fsSL --retry 3 -o "$name" "https://raw.githubusercontent.com/PX4/pyulog/$PYULOG/test/$name"
  fi
done < ../../ULOG_SHA256SUMS
sha256sum -c ../../ULOG_SHA256SUMS

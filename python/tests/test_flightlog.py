import json
import math

import pyarrow as pa
import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

import flightlog
import flightlog.compat
from conftest import external


def test_open_dataflash(mini_bin):
    log = flightlog.open(mini_bin)
    assert log.kind == "dataflash"
    assert "FMT" in log
    fmt = log.table("FMT")
    assert isinstance(fmt, pa.Table)
    assert fmt.num_rows == log.counts["FMT"]
    assert set(fmt.column_names) >= {"Type", "Length", "Name", "Format", "Columns"}
    assert len(log) == sum(log.counts.values())
    assert log.integrity["skipped_bytes"] == 0
    assert "ArduCopter" in repr(log) or "dataflash" in repr(log)


def test_open_tlog(mini_tlog):
    log = flightlog.open(mini_tlog)
    assert log.kind == "tlog"
    hb = log.table("HEARTBEAT")
    assert hb.column_names[:3] == ["timestamp_us", "sys_id", "comp_id"]
    assert hb.schema.field("timestamp_us").type == pa.uint64()
    assert log.time_column == "timestamp_us"


def test_from_bytes_matches_file(mini_bin):
    data = mini_bin.read_bytes()
    a = flightlog.open(mini_bin)
    for src in (data, bytearray(data), memoryview(data)):
        b = flightlog.from_bytes(src)
        assert b.counts == a.counts
        assert b.table("PARM").equals(a.table("PARM"))


def test_types_filter(mini_bin):
    log = flightlog.open(mini_bin, types=["PARM"])
    assert log.message_types == ["PARM"]
    assert log.counts["FMT"] > 0
    assert log.params
    with pytest.raises(KeyError):
        log.table("FMT")


def test_units_and_metadata(mini_bin):
    t = flightlog.open(mini_bin).table("PARM")
    md = t.schema.metadata
    assert md[b"flightlog.message"] == b"PARM"
    assert md[b"flightlog.format"].startswith(b"Q")


def test_polars_and_parquet(mini_bin, tmp_path):
    pl = pytest.importorskip("polars")
    log = flightlog.open(mini_bin)
    df = log.polars("PARM")
    assert isinstance(df, pl.DataFrame)
    assert df.height == log.counts["PARM"]
    out = log.to_parquet(tmp_path, names=["PARM", "FMT"])
    import pyarrow.parquet as pq

    assert pq.read_table(out["PARM"]).num_rows == log.counts["PARM"]


def test_errors(tmp_path):
    with pytest.raises(OSError):
        flightlog.open(tmp_path / "missing.bin")
    p = tmp_path / "x.bin"
    p.write_bytes(b"hello world, not a log")
    with pytest.raises(ValueError):
        flightlog.open(p)
    assert flightlog.detect(b"\xa3\x95\x80") == "dataflash"


def test_facts_schema_valid(mini_bin, mini_tlog):
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(flightlog.facts_schema())
    for p in (mini_bin, mini_tlog):
        doc = flightlog.open(p).facts()
        jsonschema.validate(doc, schema)
        assert doc["flight_facts_version"] == flightlog.FACTS_VERSION
        assert len(doc["checks"]) == 12


def test_facts_on_autotest_logs():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(flightlog.facts_schema())
    want = {
        "copter-throttle-failsafe.bin": ("rc.failsafe", "fail"),
        "copter-gcs-failsafe.bin": ("link.gcs_failsafe", "fail"),
        "copter-battery-failsafe.bin": ("battery.voltage", "fail"),
        "copter-vibration-failsafe.bin": ("vibration.failsafe", "warn"),
        "copter-gps-glitch.bin": ("gps.glitch", "fail"),
        "copter-gps-glitch.tlog": ("gps.glitch", "fail"),
    }
    for name, (check, status) in want.items():
        doc = flightlog.open(external(name)).facts()
        jsonschema.validate(doc, schema)
        c = next(c for c in doc["checks"] if c["id"] == check)
        assert c["status"] == status, (name, c["summary"])


def _py(v):
    if isinstance(v, float) and math.isnan(v):
        return "nan"
    if isinstance(v, (bytes, bytearray)):
        return bytes(v)
    if hasattr(v, "tolist"):
        return v.tolist()
    if isinstance(v, (list, tuple)):
        return [_py(x) for x in v]
    return v


def test_compat_matches_pymavlink_message_for_message():
    DFReader = pytest.importorskip("pymavlink.DFReader")
    path = str(external("copter-mission.bin"))
    ref = DFReader.DFReader_binary(path)
    ours = flightlog.compat.DFReader_binary(path)
    n = 0
    while True:
        a = ref.recv_msg()
        b = ours.recv_msg()
        if a is None or b is None:
            assert a is None and b is None
            break
        assert a.get_type() == b.get_type()
        da = {k: _py(v) for k, v in a.to_dict().items()}
        db = {k: _py(v) for k, v in b.to_dict().items()}
        if a.get_type() == "FILE":
            da["Data"] = bytes(da["Data"])
        assert da == db, (n, a.get_type())
        if a.get_type() not in ("FMT", "FMTU", "UNIT", "MULT"):
            assert abs(a._timestamp - b._timestamp) < 1e-6, (n, a.get_type(), a._timestamp, b._timestamp)
        n += 1
    assert n > 100_000
    assert ref.params == ours.params


def test_compat_recv_match_and_elements():
    DFReader = pytest.importorskip("pymavlink.DFReader")
    path = str(external("copter-mission.bin"))
    ref = DFReader.DFReader_binary(path)
    ours = flightlog.compat.DFReader_binary(path)
    for _ in range(50):
        a = ref.recv_match(type=["GPS", "XKF4"], condition="GPS.NSats >= 0")
        b = ours.recv_match(type=["GPS", "XKF4"], condition="GPS.NSats >= 0")
        assert a.get_type() == b.get_type()
        assert list(a._elements) == b._elements
    ours.rewind()
    assert ours.recv_msg().get_type() == "FMT"


def test_compat_opens_log_pymavlink_cannot():
    path = str(external("copter-mission-bitflip.bin"))
    DFReader = pytest.importorskip("pymavlink.DFReader")
    with pytest.raises(Exception):
        DFReader.DFReader_binary(path)
    ours = flightlog.compat.DFReader_binary(path)
    assert ours.integrity["invalid_formats"]
    count = 0
    while ours.recv_msg() is not None:
        count += 1
    assert count > 134_000


@settings(max_examples=200, deadline=None)
@given(st.binary(max_size=4096))
def test_random_bytes_never_crash(data):
    try:
        log = flightlog.from_bytes(data)
    except ValueError:
        return
    for name in log.message_types:
        log.table(name)
    log.facts()


@settings(max_examples=100, deadline=None)
@given(st.lists(st.tuples(st.integers(0, 81919), st.integers(0, 255)), min_size=1, max_size=40))
def test_mutated_log_never_crashes(mini_bin_bytes, edits):
    b = bytearray(mini_bin_bytes)
    for at, v in edits:
        b[at % len(b)] = v
    log = flightlog.from_bytes(bytes(b))
    for name in log.message_types:
        log.table(name)
    log.facts()


@pytest.fixture(scope="module")
def mini_bin_bytes():
    from conftest import FIXTURES

    return (FIXTURES / "mini.bin").read_bytes()

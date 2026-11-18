import json
import math
import os
import struct

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
        "plane-throttle-failsafe.bin": ("rc.failsafe", "fail"),
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


def _fmt(type_id, length, name, fmt, cols):
    return b"\xa3\x95\x80" + struct.pack("<BB4s16s64s", type_id, length, name.encode(), fmt.encode(), cols.encode())


def _msg(type_id, payload):
    return bytes([0xA3, 0x95, type_id]) + payload


FMT_SELF = _fmt(128, 89, "FMT", "BBnNZ", "Type,Length,Name,Format,Columns")


def test_parquet_names_from_the_log_stay_inside(tmp_path):
    pytest.importorskip("pyarrow.parquet")
    names = ["../X", "..\\Y", "/abc", "C:/a", "CON", "a*b", "OK_1"]
    log = FMT_SELF
    for i, n in enumerate(names):
        log += _fmt(200 + i, 3 + 12, n, "Qf", "TimeUS,V")
    for i in range(len(names)):
        for k in range(3):
            log += _msg(200 + i, struct.pack("<Qf", 1000 + k, float(i)))
    out = tmp_path / "out" / "inner"
    written = flightlog.from_bytes(log).to_parquet(str(out), names=names)
    assert set(written) == set(names)
    root = os.path.realpath(out)
    for n, p in written.items():
        assert os.path.dirname(os.path.realpath(p)) == root, (n, p)
        assert os.path.isfile(p)
    assert sorted(os.listdir(tmp_path / "out")) == ["inner"]
    assert flightlog.parquet_file_name("../X") == "%2E%2E%2FX.parquet"
    assert flightlog.parquet_file_name("CON") == "%43ON.parquet"
    assert flightlog.parquet_file_name("ATT") == "ATT.parquet"


def test_instance_needs_an_instance_field(mini_bin, mini_tlog):
    log = flightlog.open(mini_bin)
    imu = log.table("IMU")
    assert log.table("IMU", instance=0).num_rows < imu.num_rows
    assert log.table("IMU", instance=7).num_rows == 0
    with pytest.raises(ValueError, match="no instance field"):
        log.table("PARM", instance=1)
    with pytest.raises(ValueError, match="no instance field"):
        flightlog.open(mini_tlog).table("ATTITUDE", instance=3)


def test_clean_has_one_meaning(mini_bin):
    log = flightlog.open(mini_bin)
    assert log.integrity["clean"] == log.facts()["integrity"]["clean"]
    assert log.integrity["time_outliers"] == 0


def test_damaged_timestamps_do_not_stretch_the_log():
    clean = flightlog.open(external("copter-mission.bin"))
    bad = flightlog.open(external("copter-mission-bitflip.bin"))
    assert bad.time_range_us == clean.time_range_us
    assert bad.integrity["time_outliers"] > 0
    assert abs(bad.facts()["log"]["duration_s"] - clean.facts()["log"]["duration_s"]) < 2


def test_text_with_mavlink_magic_is_not_a_log(tmp_path):
    p = tmp_path / "notalog.bin"
    p.write_bytes(b"hello wo\xfdrld this is a text file" * 3)
    with pytest.raises(ValueError):
        flightlog.open(p)


def test_plane_log_facts():
    doc = flightlog.open(external("plane-throttle-failsafe.bin")).facts()
    assert doc["log"]["vehicle"] == "plane"
    status = {c["id"]: c["status"] for c in doc["checks"]}
    assert status["rc.failsafe"] == "fail"
    assert status["vibration.failsafe"] == "not_applicable"


@settings(max_examples=100, deadline=None)
@given(st.lists(st.tuples(st.integers(0, 81919), st.integers(0, 255)), min_size=1, max_size=40))
def test_mutated_log_facts_follow_the_schema(facts_schema_obj, mini_bin_bytes, edits):
    jsonschema = pytest.importorskip("jsonschema")
    b = bytearray(mini_bin_bytes)
    for at, v in edits:
        b[at % len(b)] = v
    jsonschema.validate(flightlog.from_bytes(bytes(b)).facts(), facts_schema_obj)


@settings(max_examples=100, deadline=None)
@given(st.binary(max_size=4096))
def test_random_bytes_facts_follow_the_schema(facts_schema_obj, data):
    jsonschema = pytest.importorskip("jsonschema")
    try:
        log = flightlog.from_bytes(FMT_SELF + data)
    except ValueError:
        return
    jsonschema.validate(log.facts(), facts_schema_obj)


@pytest.fixture(scope="module")
def facts_schema_obj():
    return json.loads(flightlog.facts_schema())

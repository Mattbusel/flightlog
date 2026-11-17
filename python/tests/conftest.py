import pathlib

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "flightlog-core" / "tests" / "data"
EXTERNAL = ROOT / "testdata" / "external"


@pytest.fixture
def mini_bin():
    return FIXTURES / "mini.bin"


@pytest.fixture
def mini_tlog():
    return FIXTURES / "mini.tlog"


def external(name):
    p = EXTERNAL / name
    if not p.exists():
        pytest.skip(f"{name} not downloaded (tools/fetch_testdata.sh)")
    return p

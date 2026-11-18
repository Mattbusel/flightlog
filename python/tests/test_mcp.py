import asyncio
import json

import pytest

from flightlog import mcp_server as m


@pytest.fixture
def store(mini_bin):
    return m.LogStore([str(mini_bin.parent)])


def test_overview_and_fields(store, mini_bin):
    o = m.log_overview(store, str(mini_bin))
    assert o["log"]["kind"] == "dataflash"
    assert o["message_counts"]["PARM"] > 0
    f = m.message_fields(store, "mini.bin", "PARM")
    assert [x["name"] for x in f["fields"]][:2] == ["TimeUS", "Name"]


def test_query_thins_and_windows(store, mini_bin):
    q = m.query_messages(store, str(mini_bin), "PARM", fields=["Name", "Value"], max_rows=10)
    assert q["returned"] <= 10
    assert q["rows_in_window"] > 10
    assert set(q["rows"][0]) >= {"TimeUS", "Name", "Value", "t_s"}
    w = m.query_messages(store, str(mini_bin), "PARM", start_s=1e9)
    assert w["rows_in_window"] == 0
    with pytest.raises(ValueError):
        m.query_messages(store, str(mini_bin), "PARM", fields=["Nope"])


def test_stats_params_facts(store, mini_bin):
    s = m.field_stats(store, str(mini_bin), "PARM", "Value")
    assert s["count"] > 0 and s["min"] <= s["p50"] <= s["max"]
    p = m.get_params(store, str(mini_bin), prefix="batt_")
    assert p and all(k.startswith("BATT_") for k in p)
    c = m.flight_facts(store, str(mini_bin), check="gps.hdop")
    assert c["id"] == "gps.hdop"
    assert isinstance(m.get_events(store, str(mini_bin)), list)


def test_paths_outside_roots_are_refused(store, tmp_path):
    other = tmp_path / "x.bin"
    other.write_bytes(b"\xa3\x95\x80")
    with pytest.raises(ValueError):
        m.log_overview(store, str(other))
    with pytest.raises(ValueError):
        m.log_overview(store, "../../../etc/passwd")


def test_server_lists_read_only_tools(store, mini_bin):
    pytest.importorskip("mcp")
    server = m.build_server(store)
    tools = asyncio.run(server.list_tools())
    names = {t.name for t in tools}
    assert names == {"list_logs", "log_overview", "message_fields", "query_messages", "field_stats",
                     "flight_facts", "get_params", "get_events"}
    for t in tools:
        ann = t.annotations
        assert getattr(ann, "read_only_hint", getattr(ann, "readOnlyHint", None)) is True
    res = asyncio.run(server.call_tool("flight_facts", {"path": str(mini_bin), "check": "vibration.levels"}))
    content = res.content if hasattr(res, "content") else res[0]
    text = content[0].text
    assert json.loads(text)["id"] == "vibration.levels"


def test_tool_errors_reach_the_client(store, mini_bin):
    mcp = pytest.importorskip("mcp")
    Client = getattr(mcp, "Client", None)
    if Client is None:
        pytest.skip("mcp.Client (in-process client) needs mcp 2")
    server = m.build_server(store)

    async def go():
        async with Client(server) as c:
            bad_msg = await c.call_tool("query_messages", {"path": str(mini_bin), "message": "NOPE"})
            bad_field = await c.call_tool("query_messages", {"path": str(mini_bin), "message": "PARM", "fields": ["Nope"]})
            outside = await c.call_tool("log_overview", {"path": "../../../etc/passwd"})
            return bad_msg, bad_field, outside

    bad_msg, bad_field, outside = asyncio.run(go())
    for res in (bad_msg, bad_field, outside):
        assert getattr(res, "is_error", getattr(res, "isError", None)) is True
    assert "NOPE" in bad_msg.content[0].text
    assert "Nope" in bad_field.content[0].text and "Value" in bad_field.content[0].text
    assert "outside the allowed directories" in outside.content[0].text


def test_window_rejects_non_finite_times(store, mini_bin):
    with pytest.raises(ValueError, match="finite"):
        m.query_messages(store, str(mini_bin), "PARM", start_s=float("nan"))

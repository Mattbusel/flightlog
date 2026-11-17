import json

from flightlog import cli


def test_cli_facts_and_schema(mini_bin, capsys, tmp_path):
    assert cli.main(["schema"]) == 0
    assert json.loads(capsys.readouterr().out)["title"] == "flight-facts v1"
    cli.main(["facts", str(mini_bin)])
    assert json.loads(capsys.readouterr().out)["log"]["kind"] == "dataflash"
    assert cli.main(["export", str(mini_bin), str(tmp_path)]) == 0
    assert (tmp_path / "PARM.parquet").exists()
    assert cli.main(["info", str(mini_bin)]) == 0

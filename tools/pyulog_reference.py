"""Reference summary of a ULog file as pyulog reads it.

    pip install pyulog==1.3.0
    python tools/pyulog_reference.py LOG.ulg > LOG.ulg.pyulog.json

For every topic (all multi_id instances together): the row count, the field
names, and per field the sum of all values (ints summed exactly, as a
string; floats as repr() of the float sum, NaN values left out). Padding
fields are left out. Also the parameters at the end of the log, the number
of parameter changes, logged messages and dropouts, and the info keys.
tests/ulog_reference.rs computes the same from flightlog-core.
"""
import json
import math
import sys

import numpy as np
from pyulog import ULog


def main(path):
    u = ULog(path)
    topics = {}
    for d in u.data_list:
        t = topics.setdefault(d.name, {"count": 0, "fields": {}})
        t["count"] += len(d.data["timestamp"])
        for f in d.field_data:
            name = f.field_name
            if name.startswith("_padding"):
                continue
            v = d.data[name]
            acc = t["fields"].setdefault(name, None)
            if np.issubdtype(v.dtype, np.floating):
                s = float(np.nansum(v.astype(np.float64)))
                t["fields"][name] = repr((float(acc) if acc is not None else 0.0) + s)
            else:
                s = int(sum(int(x) for x in v.tolist()))
                t["fields"][name] = str((int(acc) if acc is not None else 0) + s)
    params = dict(u.initial_parameters)
    for _, k, v in u.changed_parameters:
        params[k] = v
    out = {
        "pyulog": "1.3.0",
        "topics": topics,
        "params": {k: (float(v) if not isinstance(v, int) else v) for k, v in params.items()},
        "changed_params": len(u.changed_parameters),
        "logged": len(u.logged_messages)
        + sum(len(v) for v in u.logged_messages_tagged.values()),
        "dropouts": len(u.dropouts),
        "info_keys": sorted(u.msg_info_dict.keys()),
        "file_corrupt": bool(u.file_corruption),
    }
    json.dump(out, sys.stdout, indent=1, sort_keys=True)


if __name__ == "__main__":
    main(sys.argv[1])

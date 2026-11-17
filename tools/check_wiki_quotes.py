"""Checks that every wiki quote in flightlog-core/src/facts/wiki.rs is still
verbatim in the ArduPilot wiki source, at the pinned commit and (with
--latest) on master.

    python tools/check_wiki_quotes.py [--latest]
"""
import pathlib
import re
import sys
import urllib.request

SRC = pathlib.Path(__file__).resolve().parent.parent / "flightlog-core" / "src" / "facts" / "wiki.rs"


def unrust(s):
    # Rust string escapes used in wiki.rs: \" and \\
    return s.replace('\\"', '"').replace("\\\\", "\\")


def main():
    text = SRC.read_text(encoding="utf-8")
    commit = re.search(r'WIKI_COMMIT: &str = "([0-9a-f]{40})"', text).group(1)
    cites = re.findall(r'file: "([^"]+)",\s*quote: "((?:[^"\\]|\\.)*)"', text)
    refs = [commit] + (["master"] if "--latest" in sys.argv else [])
    bad = 0
    cache = {}
    for ref in refs:
        for file, quote in cites:
            key = (ref, file)
            if key not in cache:
                url = f"https://raw.githubusercontent.com/ArduPilot/ardupilot_wiki/{ref}/{file}"
                with urllib.request.urlopen(url, timeout=30) as r:
                    cache[key] = r.read().decode("utf-8")
            # RST wraps lines; the published page collapses whitespace too
            q = " ".join(unrust(quote).split())
            if q not in " ".join(cache[key].split()):
                bad += 1
                print(f"MISSING at {ref}: {file}: {q[:80]}")
    print(f"{len(cites)} quotes x {len(refs)} refs checked, {bad} missing")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate geocentric lunar illumination references; offline replay unless --fetch is explicit.

Uses the pinned phase-0 environment. Does not replace phase-0 model regressions or their audit metadata.
"""
import argparse
import hashlib
import json
from pathlib import Path
from generate import ROOT, query_horizons, read_observer_rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true")
    args = parser.parse_args()
    directory = ROOT / "tests/fixtures/reference"
    epochs = [2451545.0, 2459146.0, 2460736.9583333335, 2451550.2597]
    parameters = {"COMMAND": "301", "MAKE_EPHEM": "YES", "EPHEM_TYPE": "OBSERVER", "CENTER": "500@399",
                  "TLIST": " ".join(map(str, epochs)), "TLIST_TYPE": "JD", "TIME_TYPE": "TT",
                  "QUANTITIES": "10,43", "APPARENT": "AIRLESS", "EXTRA_PREC": "YES", "CSV_FORMAT": "YES",
                  "CAL_FORMAT": "JD"}
    raw, digest = query_horizons(directory, "moon_phase_geocentric", parameters, args.fetch)
    output = {"generator": "scripts/reference/moon_phase.py", "generator_version": 1,
              "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "response_sha256": digest,
              "conventions": "Horizons geocentric Earth observer, TT Julian dates, quantity 10 Illu% / 100; includes apparent-place effects absent from the current geometric model",
              "rows": read_observer_rows(raw)}
    (directory / "moon_phase.json").write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()

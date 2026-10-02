#!/usr/bin/env python3
"""Generate auditable ERFA/Horizons fixtures. No network unless --fetch is supplied.

Raw responses and complete request parameters are retained; default runs replay those responses.
The probe measures the current Rust model and is never labelled as an independent reference.
"""

import argparse
import csv
import hashlib
import importlib.metadata
import io
import json
import math
from pathlib import Path
import struct
import subprocess
import urllib.parse
import urllib.request

import erfa
import numpy as np

ROOT = Path(__file__).resolve().parents[2]
VERSION = 1
BOSTON_UTC = 2459146.0
BOSTON_TT = BOSTON_UTC + 69.184 / 86400.0
SITE = {"latitude_deg": 42.3601, "longitude_deg": -71.0589, "height_m": 0.0}
URL = "https://ssd.jpl.nasa.gov/api/horizons.api"


def query_horizons(directory, name, parameters, fetch):
    """Fetch once or replay a raw response, checking that it belongs to these exact parameters."""
    request_path = directory / f"{name}.request.json"
    response_path = directory / f"{name}.response.json"
    parameters = {"format": "json", **{key: f"'{value}'" for key, value in parameters.items()}}
    if fetch:
        url = URL + "?" + urllib.parse.urlencode(parameters)
        with urllib.request.urlopen(url, timeout=60) as response:
            raw = response.read()
        result = json.loads(raw)
        if "error" in result or "$$SOE" not in result.get("result", ""):
            raise RuntimeError(f"{name}: {result.get('error', result.get('result', result))}")
        request_path.write_text(json.dumps({"url": URL, "parameters": parameters}, indent=2) + "\n")
        response_path.write_bytes(raw)
    else:
        request = json.loads(request_path.read_text())
        if request != {"url": URL, "parameters": parameters}:
            raise ValueError(f"Request changed: regenerate {name} with --fetch")
        raw = response_path.read_bytes()
        result = json.loads(raw)
    return result["result"], hashlib.sha256(raw).hexdigest()


def read_observer_rows(text):
    before, rest = text.split("$$SOE", 1)
    header = next(line for line in reversed(before.splitlines()) if "Date__" in line)
    fields = next(csv.reader([header]))
    rows = csv.reader(io.StringIO(rest.split("$$EOE", 1)[0].strip()))
    return [{key.strip(): value.strip() for key, value in zip(fields, row) if key.strip()} for row in rows]


def read_vector_rows(text):
    rows = csv.reader(io.StringIO(text.split("$$SOE", 1)[1].split("$$EOE", 1)[0].strip()))
    return [{"jd_tdb": float(row[0]), "position_au": list(map(float, row[2:5])),
             "velocity_au_day": list(map(float, row[5:8]))} for row in rows]


def build_erfa_references():
    """Near-date horizontal references use BSC5 inputs and ERFA's documented current-era routines."""
    raw = (ROOT / "data/bsc5").read_bytes()
    stars = []
    for hr, name in [(7001, "Vega"), (5340, "Arcturus")]:
        offset = 28 + (hr - 1) * 32
        ra, dec = struct.unpack_from("<dd", raw, offset + 4)
        pmra, pmdec = struct.unpack_from("<ff", raw, offset + 24)
        apparent = erfa.atco13(ra, dec, pmra, pmdec, 0.0, 0.0, BOSTON_UTC, 0.0, 0.0,
                              math.radians(SITE["longitude_deg"]), math.radians(SITE["latitude_deg"]),
                              0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.55)
        years = (BOSTON_UTC - 2451545.0) / 365.2425
        mean_vector = erfa.pmat06(2451545.0, BOSTON_UTC - 2451545.0) @ erfa.s2c(ra + pmra * years, dec + pmdec * years)
        mean_ra, mean_dec = erfa.c2s(mean_vector)
        hour_angle = erfa.gmst06(BOSTON_UTC, 0.0, BOSTON_UTC, 0.0) + math.radians(SITE["longitude_deg"]) - mean_ra
        az, alt = erfa.hd2ae(hour_angle, mean_dec, math.radians(SITE["latitude_deg"]))
        stars.append({"name": name, "hr": hr, "input_ra_dec_pm_rad": [ra, dec, pmra, pmdec],
                      "apparent_az_alt_rad": [float(apparent[0]), float(math.pi / 2 - apparent[1])],
                      "matched_legacy_mean_az_alt_rad": [float(az), float(alt)]})
    precession = []
    for year in [5026.0, 7026.0, 9026.0, 12026.0, 15026.0]:
        first, second = erfa.epj2jd(year)
        long_term = erfa.ltp(year)
        # pmat06 includes frame bias; remove its J2000 value before comparing precession-only rotations.
        iau = erfa.pmat06(first, second) @ erfa.pmat06(2451545.0, 0.0).T
        delta = float(np.linalg.norm(erfa.rm2v(iau @ long_term.T)) * erfa.DR2AS / 60.0)
        precession.append({"julian_epoch_tt": year, "ltp_matrix": long_term.tolist(), "iau06_vs_ltp_arcmin": delta})
    return {"boston_stars": stars, "precession": precession,
            "conventions": {"stars": "atco13, ICRS BSC5 inputs, UTC JD 2459146, DUT1=xp=yp=0, zero parallax/radial velocity, WGS84 height 0, pressure 0; includes standard apparent-place corrections",
                            "matched_legacy": "linear RA/Dec with legacy 365.2425-day year; pmat06 and hd2ae, TT=UT1; no refraction/aberration/nutation",
                            "precession": "eraLtp versus eraPmat06 with J2000 bias removed; TT Julian epochs; rotation angle"}}


def generate_long_term_stars(earth, sun):
    """Synthetic ICRS star: explicit components, no epv00 or high-level apparent-place routine at distant epochs."""
    initial = [1.0, 0.5, 1e-5, -2e-5, 0.1, 20.0]  # RA, Dec, dRA/dyr, dDec/dyr, parallax arcsec, radial velocity km/s
    rows = []
    for earth_row, sun_row in zip(earth, sun, strict=True):
        jd = earth_row["jd_tdb"]  # TT ≈ TDB for these reference epochs; explicitly recorded below
        epoch = erfa.epj(jd, 0.0)
        moved = erfa.starpm(*initial, 2451545.0, 0.0, jd, 0.0)
        direction = erfa.s2c(moved[0], moved[1])
        beta = np.array(earth_row["velocity_au_day"]) / erfa.DC
        sun_distance = np.linalg.norm(np.array(earth_row["position_au"]) - np.array(sun_row["position_au"]))
        aberrated = erfa.ab(direction, beta, sun_distance, math.sqrt(1.0 - beta @ beta))
        dpsi, deps = erfa.nut00b(jd, 0.0)
        obliquity = math.acos(np.clip(erfa.ltpecl(epoch) @ erfa.ltpequ(epoch), -1.0, 1.0))
        observed = erfa.numat(obliquity, dpsi, deps) @ erfa.ltpb(epoch) @ aberrated
        rows.append({"jd_tt_approximately_tdb": jd, "space_motion_ra_dec": [float(moved[0]), float(moved[1])],
                     "apparent_direction_of_date": observed.tolist()})
    return {"input_synthetic_icrs": initial,
            "recipe": "starpm -> ab (Horizons barycentric Earth velocity, Sun distance) -> ltpb -> nut00b/numat; mean obliquity from ltpecl/ltpequ; no stellar parallax or light deflection; TT approximated by TDB; model-comparison fixtures, not validated predictions",
            "rows": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true", help="Query Horizons and replace saved raw responses")
    parser.add_argument("--probe", type=Path, help="Built examples/reference_probe executable for the current-model audit")
    parser.add_argument("--output", type=Path, default=ROOT / "tests/fixtures/reference")
    parser.add_argument("--record-current", action="store_true", help="Explicitly replace current-model numeric regression fixtures")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    result = {"generator_version": VERSION, "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "versions": {name: importlib.metadata.version(name) for name in ["pyerfa", "numpy", "astropy", "astroquery"]},
              "site": SITE, "boston_utc_jd": BOSTON_UTC, "boston_tt_jd": BOSTON_TT, **build_erfa_references()}
    bodies = []
    for name, target in [("Sun", "10"), ("Mars", "499"), ("Neptune", "899"), ("Moon", "301")]:
        parameters = {"COMMAND": target, "MAKE_EPHEM": "YES", "EPHEM_TYPE": "OBSERVER", "CENTER": "coord@399",
                      "COORD_TYPE": "GEODETIC", "SITE_COORD": "-71.0589,42.3601,0", "TLIST": str(BOSTON_TT),
                      "TLIST_TYPE": "JD", "TIME_TYPE": "TT", "QUANTITIES": "2,4,10,20,43", "APPARENT": "AIRLESS",
                      "ANG_FORMAT": "DEG", "EXTRA_PREC": "YES", "CSV_FORMAT": "YES", "CAL_FORMAT": "JD"}
        raw, digest = query_horizons(args.output, f"boston_{name.lower()}", parameters, args.fetch)
        bodies.append({"name": name, "response_sha256": digest, "rows": read_observer_rows(raw)})
    result["horizons_boston"] = bodies
    vectors = {}
    # Horizons exposes only the interval through AD 9999, despite DE441's longer underlying span.
    # Never substitute a different body or extrapolate reference velocities to make an unavailable query pass.
    epochs = " ".join(str(2451545.0 + (year - 2000.0) * 365.25) for year in [2026, 4026, 8026])
    for name, target in [("earth", "399"), ("sun", "10")]:
        parameters = {"COMMAND": target, "MAKE_EPHEM": "YES", "EPHEM_TYPE": "VECTORS", "CENTER": "500@0",
                      "REF_PLANE": "FRAME", "REF_SYSTEM": "ICRF", "OUT_UNITS": "AU-D", "VEC_TABLE": "2",
                      "VEC_CORR": "NONE", "TLIST": epochs, "TLIST_TYPE": "JD", "TIME_TYPE": "TDB", "CSV_FORMAT": "YES"}
        raw, digest = query_horizons(args.output, f"{name}_vectors", parameters, args.fetch)
        vectors[name] = {"response_sha256": digest, "rows": read_vector_rows(raw)}
    result["barycentric_vectors"] = vectors
    result["unavailable_references"] = [{"year": 12026, "quantity": "Horizons state/apparent place",
                                          "reason": "Horizons rejects dates after AD 9999; a separately obtained DE441 kernel is needed. ERFA long-term precession references are still generated."}]
    result["long_term_star"] = generate_long_term_stars(vectors["earth"]["rows"], vectors["sun"]["rows"])
    if args.probe:
        output = subprocess.check_output([str(args.probe.resolve())], text=True)
        probe_rows = list(csv.reader(io.StringIO(output)))
        result["current_model_probe"] = probe_rows
        if args.record_current:
            positions = [row for row in probe_rows if row[0] == "position"]
            source = "// Current-model regression baseline, not an independent accuracy reference.\n// Generated by scripts/reference/generate.py --record-current; Boston observer, UTC=UT1=TT approximation.\n"
            source += "pub const POSITIONS: &[(f64, &str, f64, f64)] = &[\n"
            source += "".join(f'    ({float(row[1])!r}, "{row[2]}", {float(row[3])!r}, {float(row[4])!r}),\n' for row in positions)
            source += "];\n"
            (args.output.parent / "current_positions.rs").write_text(source)
    (args.output / "references.json").write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
    print(json.dumps({"precession": result["precession"], "boston_stars": result["boston_stars"], "horizons_boston": bodies}, indent=2))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Explicitly replace the phase-6 numerical regression baseline; no independent fixture is modified."""
import csv,io,subprocess
from pathlib import Path
from generate import ROOT
output=subprocess.check_output([str(ROOT/'target/release/examples/reference_probe')],text=True)
rows=[r for r in csv.reader(io.StringIO(output)) if r[0]=='position']
source='// Phase-6 regression baseline, not an independent reference.\n// scripts/reference/record_current.py; Boston, UTC input, Espenak–Meeus TT, WGS84, apparent airless.\n'
source+='pub const POSITIONS: &[(f64, &str, f64, f64)] = &[\n'
source+=''.join(f'    ({float(r[1])!r}, "{r[2]}", {float(r[3])!r}, {float(r[4])!r}),\n' for r in rows)
source+='];\n'
(ROOT/'tests/fixtures/phase6_positions.rs').write_text(source)

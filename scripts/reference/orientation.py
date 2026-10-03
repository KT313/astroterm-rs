#!/usr/bin/env python3
"""Offline ERFA contemporary apparent-sidereal and full-rotation references (pyerfa 2.0.1.5)."""
import hashlib,json
from pathlib import Path
import erfa
from generate import ROOT
rows=[]
for tt in [2415020.,2451545.,2459146.000800741,2461041.5,2488070.]:
    ut1=tt-69.184/86400
    rows.append({'tt':tt,'ut1':ut1,'gast':float(erfa.gst06a(ut1,0,tt,0)),'c2i06a':erfa.c2i06a(tt,0).tolist()})
(ROOT/'tests/fixtures/reference/orientation.json').write_text(json.dumps({'pyerfa':erfa.__version__,
    'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    'source':'ERFA gst06a and c2i06a; radians, TT/UT1 separately supplied; includes frame bias', 'rows':rows},indent=2)+'\n')

#!/usr/bin/env python3
"""Replay/fetch Horizons body-center apparent, airless topocentric checks with TDB−UTC and UT1−UTC explicitly saved."""
import argparse,json,hashlib
from pathlib import Path
from generate import query_horizons,read_observer_rows,ROOT
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--fetch',action='store_true')
a=p.parse_args()
d=ROOT/'tests/fixtures/reference'
rows=[]
for name,target in [('Sun',10),('Mercury',199),('Venus',299),('Mars',499),('Jupiter',599),('Saturn',699),('Uranus',799),('Neptune',899),('Moon',301)]:
    params={'COMMAND':str(target),'MAKE_EPHEM':'YES','EPHEM_TYPE':'OBSERVER','CENTER':'coord@399',
        'COORD_TYPE':'GEODETIC','SITE_COORD':'-71.0589,42.3601,0','TLIST':'2459146.000800741 2461041.5 2461223.5',
        'TLIST_TYPE':'JD','TIME_TYPE':'TT','QUANTITIES':'2,4,30,49','APPARENT':'AIRLESS','ANG_FORMAT':'DEG',
        'EXTRA_PREC':'YES','CSV_FORMAT':'YES','CAL_FORMAT':'JD'}
    raw,digest=query_horizons(d,'phase6_'+name.lower(),params,a.fetch)
    rows.append({'name':name,'response_sha256':digest,'rows':read_observer_rows(raw)})
(d/'topocentric.json').write_text(json.dumps({'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    'site':'WGS84 Boston, sea level, airless; quantities 30 and 49 supply TDB−UTC and UT1−UTC; TT≈TDB for matched rotation', 'bodies':rows},indent=2)+'\n')

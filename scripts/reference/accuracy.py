#!/usr/bin/env python3
"""Phase-6 independent DE441 + ERFA component audit, matched TT and UT1.

Offline by default. --fetch retrieves only small DE441 byte ranges, stored with SHA256 and HTTP provenance.
Outer-planet states are explicitly system barycenters: their center offsets are not certified by this audit.
No extrapolation of reference data and no epv00 outside 1900–2100.
"""
import argparse
import hashlib
import importlib.metadata
import json
import math
from pathlib import Path
import subprocess
import sys
import erfa
import numpy as np
from de441 import Kernel

ROOT = Path(__file__).resolve().parents[2]
BODY_IDS = [10,199,299,399,4,5,6,7,8,301]
NAMES = ['Sun','Mercury','Venus','Earth','Mars','Jupiter','Saturn','Uranus','Neptune','Moon']
SITE = np.radians([-71.0589,42.3601])

def rotate_z(a):
    return np.array([[np.cos(a),np.sin(a),0],[-np.sin(a),np.cos(a),0],[0,0,1]])

def slow_orientation(tt):
    epoch = erfa.epj(tt,0)
    # Independent Gaussian quadrature of the CIO parallel-transport integral, using ERFA's equator poles.
    nodes,weights = np.polynomial.legendre.leggauss(64)
    t = (epoch-2000)/100
    ep = 2000+(nodes+1)*t*50
    poles = erfa.ltpequ(ep)
    derivative = (erfa.ltpequ(ep+0.01)-erfa.ltpequ(ep-0.01))*5000
    integral = np.sum(weights*(poles[:,0]*derivative[:,1]-poles[:,1]*derivative[:,0])/(1+poles[:,2]))*t/2
    x,y,z = erfa.ltpequ(epoch)
    a = 1/(1+z)
    basis = np.array([[1-a*x*x,-a*x*y,-x],[-a*x*y,1-a*y*y,-y],[x,y,z]])
    cio = rotate_z(integral+0.014506/erfa.DR2AS) @ basis
    p = erfa.ltp(epoch)
    relation = cio @ p.T
    eo = -math.atan2(relation[0,1],relation[0,0])
    eps = math.acos(np.clip(erfa.ltpequ(epoch)@erfa.ltpecl(epoch),-1,1))
    psi,deps = erfa.nut00b(tt,0)
    return rotate_z(-eo+psi*math.cos(eps)) @ erfa.numat(eps,psi,deps) @ erfa.ltpb(epoch)

def build_reference(k, tt, ut1):
    obs_tt = tt+59/86400
    obs_ut1 = ut1+59/86400
    lon,lat = SITE
    rot = rotate_z(erfa.era00(obs_ut1,0)) @ slow_orientation(obs_tt)
    horizon = np.array([[-np.sin(lon),np.cos(lon),0],[-np.sin(lat)*np.cos(lon),-np.sin(lat)*np.sin(lon),np.cos(lat)],
                        [np.cos(lat)*np.cos(lon),np.cos(lat)*np.sin(lon),np.sin(lat)]])
    site = erfa.gd2gc(1,lon,lat,0)/149597870700
    omega = 2*np.pi*1.00273781191135448
    site_velocity = np.cross([0,0,omega],site)
    earth,ev = k.state(399,obs_tt)
    observer = earth+rot.T@site
    velocity = ev+rot.T@site_velocity
    beta = velocity/erfa.DC
    sun = k.state(10,obs_tt)[0]
    sun_distance = np.linalg.norm(sun-observer)
    def aberrate(v):
        v=v/np.linalg.norm(v)
        return erfa.ab(v,beta,sun_distance,math.sqrt(1-beta@beta))
    bodies=[]
    for name,target in zip(NAMES,BODY_IDS):
        if name=='Earth': continue
        emission=obs_tt
        for _ in range(3):
            p,_ = k.state(target,emission)
            emission=obs_tt-np.linalg.norm(p-observer)/erfa.DC
        direction = horizon @ rot @ aberrate(k.state(target,emission)[0]-observer)
        bodies.append([name,direction.tolist()])
    stars=[]
    import struct
    raw=(ROOT/'data/bsc5').read_bytes()
    for hr in [7001,5340]:
        offset=28+(hr-1)*32
        ra,dec=struct.unpack_from('<dd',raw,offset+4)
        pmra,pmdec=struct.unpack_from('<ff',raw,offset+24)
        u=erfa.s2c(ra,dec)
        w=pmra*np.cos(dec)*np.array([-np.sin(ra),np.cos(ra),0])+pmdec*np.array([-np.sin(dec)*np.cos(ra),-np.sin(dec)*np.sin(ra),np.cos(dec)])
        stars.append((horizon @ rot @ aberrate(u+w*(obs_tt-2451545)/365.25)).tolist())
    return {'tt':tt,'ut1':ut1,'states':[np.concatenate(k.state(target,tt)).tolist() for target in BODY_IDS],
            'precession':erfa.ltp(erfa.epj(tt,0)).tolist(),'nutation':list(erfa.nut00b(tt,0)),
            'slow':slow_orientation(tt).tolist(),'bodies':bodies,'stars':stars}

def separation(a,b):
    return math.atan2(np.linalg.norm(np.cross(a,b)),np.dot(a,b))*erfa.DR2AS

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--fetch',action='store_true')
    p.add_argument('--ranges',type=Path,default=ROOT/'dev/phase6-de441-ranges')
    p.add_argument('--probe',type=Path,default=ROOT/'target/release/examples/accuracy_probe')
    p.add_argument('--full',action='store_true')
    p.add_argument('--output',type=Path,default=ROOT/'tests/fixtures/reference/accuracy.json')
    args=p.parse_args()
    years=[-7974,-2000,0,1900,2026,4026,8026,12026]
    epochs=[2451545.+(y-2000)*365.25 for y in years]
    if args.full:
        epochs += [2451545.+(y-2000)*365.25+d for y in range(1800,2201,25) for d in (0,91,182,273)]
        epochs += [2451545.+(y-2000)*365.25+d for y in (1900,2000,2020,2026) for d in range(0,365,15)]
        epochs += [2451545.+(y-2000)*365.25+d for y in range(0,4001,250) for d in (0,183)]
        epochs += [2451545.+(y-2000)*365.25 for y in range(-7000,12001,1000)]
        epochs += [2451545.+(y-2000)*365.25+d for y in range(1850,2031,5) for d in range(0,365,30)]
        epochs += [-1191383.5,6113831.5-1/86400,2396758.5,2462502.5,1721059.5,3182029.5]
    epochs=sorted(set(epochs))
    kernel=Kernel(args.ranges,args.fetch)
    kernel.prepare(epochs)
    rows=[build_reference(kernel,tt,tt) for tt in epochs] # fixed equal TT/UT1 isolates models, not a time-scale assumption
    result={'generator_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'kernel_reader_sha256':hashlib.sha256(Path(__file__).with_name('de441.py').read_bytes()).hexdigest(),
        'versions':{n:importlib.metadata.version(n) for n in ['pyerfa','numpy','jplephem']},
        'source':'JPL DE441 direct SPK; TT approximated by TDB (<2 ms near today)',
        'site':'WGS84 Boston, -71.0589 E, 42.3601 N, height 0 m; no refraction/polar motion/light deflection',
        'corrections':'3 light-time iterations; ERFA ab + ltpb + numat/nut00b; ERA UT1, model-consistent CIO via 64-node Gaussian integral',
        'outer_planets':'Mars/Jupiter/Saturn/Uranus/Neptune system barycenters; center offsets require separate Horizons checks',
        'frame_time_offset_seconds':59,'rows':rows,
        'ranges':[json.loads(path.read_text()) for path in sorted(kernel.remote.used)]}
    identities={(r['etag'],r['last_modified']) for r in result['ranges']}
    assert len(identities)==1, 'mixed kernel versions: refetch into a clean range directory'
    args.output.write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
    probe=subprocess.check_output([str(args.probe)],input=''.join(f'{tt} {tt}\n' for tt in epochs),text=True)
    actual=[json.loads(line) for line in probe.splitlines()]
    maxima={}
    qualified={}
    for reference,got in zip(rows,actual,strict=True):
        year=erfa.epj(reference['tt'],0)
        distance=abs(year-2000)
        band=('past' if year<2000 else 'future')+('-near' if distance<=200 else '-middle' if distance<=2000 else '-far')
        errors={name:separation(v,dict(got['bodies'])[name]) for name,v in reference['bodies']}
        errors['Stars']=max(separation(a,b) for a,b in zip(reference['stars'],got['stars']))
        errors['Precession']=np.linalg.norm(erfa.rm2v(np.array(got['precession'])@np.array(reference['precession']).T))*erfa.DR2AS
        errors['Orientation']=np.linalg.norm(erfa.rm2v(np.array(got['slow'])@np.array(reference['slow']).T))*erfa.DR2AS
        for name,error in errors.items():
            if name not in ('Precession','Orientation'):
                family=0 if name=='Stars' else 2 if name=='Moon' else 1
                if got['coverage'][family]:
                    assert error < got['targets'][family], f'claimed coverage fails: {year} {name}: {error}'
                    qualified[name]=max(qualified.get(name,0),error)
            key=band+'/'+name
            if key not in maxima or error>maxima[key]['arcsec']:
                maxima[key]={'arcsec':error,'year':year}
        print(year, {name:round(error,3) for name,error in errors.items()},flush=True)
    report={'sample_count':len(rows),'maxima':maxima,'qualified_maxima_arcsec':qualified}
    (ROOT/'dev/phase6-accuracy-results.json').write_text(json.dumps(report,indent=2)+'\n')
    (ROOT/'dev/phase6-probe.json').write_text(json.dumps(actual,indent=2)+'\n')
    print(json.dumps(report,indent=2))
if __name__=='__main__': main()

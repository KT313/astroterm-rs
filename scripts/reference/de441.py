#!/usr/bin/env python3
"""Read only required DE441 Chebyshev records through verified HTTP ranges.

Raw byte ranges and SHA256 metadata are replayable locally; the 3 GB kernel is never downloaded in full.
Independent reference tool only (jplephem 2.24 parses the DAF directory; numpy evaluates Chebyshev series).
"""
import hashlib
import json
import math
from pathlib import Path
import urllib.request
from concurrent.futures import ThreadPoolExecutor
import numpy as np
from jplephem.daf import DAF

URL = 'https://ssd.jpl.nasa.gov/ftp/eph/planets/bsp/de441.bsp'
AU_KM = 149597870.7

class RangeFile:
    def __init__(self, directory, fetch):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.fetch = fetch
        self.offset = 0
        self.used = set()

    def seek(self, offset, whence=0):
        assert whence == 0
        self.offset = offset

    def read(self, size):
        offset = self.offset
        self.offset += size
        return self.read_range(offset, size)

    def read_range(self, offset, size):
        path = self.directory / f'{offset}-{size}.bin'
        meta = path.with_suffix('.json')
        if not path.exists():
            if not self.fetch:
                raise FileNotFoundError(f'{path}; use --fetch to obtain the recorded JPL range')
            request = urllib.request.Request(URL, headers={'Range': f'bytes={offset}-{offset+size-1}'})
            with urllib.request.urlopen(request, timeout=120) as response:
                assert response.status == 206, 'server must honor ranges; refuse full kernel'
                assert response.headers['Content-Range'].startswith(f'bytes {offset}-{offset+size-1}/')
                data = response.read(size+1)
                assert len(data) == size
                info = {'url': URL, 'offset': offset, 'length': size, 'sha256': hashlib.sha256(data).hexdigest(),
                        'last_modified': response.headers.get('Last-Modified'), 'etag': response.headers.get('ETag')}
            path.write_bytes(data)
            meta.write_text(json.dumps(info, indent=2)+'\n')
        self.used.add(meta)
        data = path.read_bytes()
        info = json.loads(meta.read_text())
        assert info['url'] == URL and info['offset'] == offset and info['length'] == len(data) == size
        assert hashlib.sha256(data).hexdigest() == info['sha256']
        return data

class Kernel:
    def __init__(self, directory, fetch):
        self.remote = RangeFile(directory, fetch)
        daf = DAF(self.remote)
        self.endian = daf.endian
        self.segments = {}
        for name, values in daf.summaries():
            t0,t1,target,center,frame,kind,start,end = values
            assert frame == 1 and kind == 2
            init,span,rsize,count = daf.read_array(end-3,end)
            self.segments[center,target,start] = (start,init,span,int(rsize),int(count),name.decode())
        self.blocks = {}

    def prepare(self, epochs):
        requests = {}
        for (center,target,key),(start,init,span,rsize,count,name) in self.segments.items():
            indices = set()
            for jd in epochs:
                i = math.floor(((jd-2451545.0)*86400.0-init)/span)
                if not 0 <= i < count: continue
                # Up to a day of retarded evaluation and the +59-second cached observation.
                for j in [i-1,i,i+1]:
                    if 0 <= j < count: indices.add(j)
            # Merge nearby records: dense contemporary samples become a few modest downloads.
            groups = []
            for i in sorted(indices):
                if groups and i-groups[-1][1] <= 16:
                    groups[-1][1] = i
                else:
                    groups.append([i,i])
            for lo,hi in groups:
                offset = (start-1+rsize*lo)*8
                size = (hi-lo+1)*rsize*8
                requests[offset,size] = (center,target,key,lo,hi,rsize)
        def fetch(item):
            (offset,size), spec = item
            data = self.remote.read_range(offset,size)
            return spec,np.frombuffer(data,dtype=self.endian+'f8').reshape(-1,spec[-1])
        with ThreadPoolExecutor(max_workers=6) as pool:
            for (center,target,key,lo,hi,rsize),data in pool.map(fetch,requests.items()):
                for j in range(lo,hi+1): self.blocks[center,target,key,j] = data[j-lo]
        print(f'DE441: {len(requests)} ranges, {sum(s for _,s in requests)/1e6:.1f} MB', flush=True)

    def relative(self, center, target, jd):
        seconds = (jd-2451545.0)*86400.0
        segment = next(s for (c,t,key),s in self.segments.items() if (c,t)==(center,target) and s[1] <= seconds < s[1]+s[2]*s[4])
        key,init,span,rsize,count,_ = segment
        i = math.floor((seconds-init)/span)
        record = self.blocks[center,target,key,i]
        midpoint,radius = record[:2]
        coefficients = record[2:].reshape(3,-1)
        x = (seconds-midpoint)/radius
        p = np.array([np.polynomial.chebyshev.chebval(x,c) for c in coefficients])/AU_KM
        v = np.array([np.polynomial.chebyshev.chebval(x,np.polynomial.chebyshev.chebder(c)) for c in coefficients])*86400.0/radius/AU_KM
        return p,v

    def state(self, target, jd):
        if target in (301,399):
            a,b = self.relative(0,3,jd)
            c,d = self.relative(3,target,jd)
            return a+c,b+d
        if target in (199,299):
            a,b = self.relative(0,target//100,jd)
            c,d = self.relative(target//100,target,jd)
            return a+c,b+d
        return self.relative(0,target,jd)

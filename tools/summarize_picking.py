#!/usr/bin/env python3
"""Compare direct live-column and runtime-object geometry access in click replays."""
import argparse
import csv
from pathlib import Path
import re
import statistics

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory',type=Path)
args=parser.parse_args()
rows=[]
hashes=set()
viewports=set()
for path in sorted(args.directory.glob('pair-*/*.log')):
    text=path.read_text()
    assert 'STAGE,pick,end' in text, f'incomplete run: {path}'
    active=text.split('STAGE,pick,start\n')[1].split('STAGE,pick,end')[0]
    pair,geometry=path.parent.name.removeprefix('pair-').split('-',1)
    clicks=sorted(float(v) for v in re.findall(r'^pick_down_ms,([0-9.]+)',active,re.M))
    assert len(clicks)>=100, f'too few click samples: {path}'
    frames=len(re.findall(r'^FRAME,',active,re.M))
    row=dict(pair=int(pair),geometry=geometry,clicks=len(clicks),fps=frames/8,
             down_p50_ms=statistics.median(clicks),down_p95_ms=clicks[round((len(clicks)-1)*.95)])
    for phase in ['handles','reconnect']:
        # Reports normalize totals over 30 frames, with explicit call counts.
        totals=[(float(ms)*30,int(calls)) for ms,calls in re.findall(r'\n  '+phase+r'\s+([0-9.]+) ms/frame  \((\d+) calls\)',active)]
        row[phase+'_mean_ms']=sum(ms for ms,n in totals)/sum(n for ms,n in totals)
    rows.append(row)
    hashes.add((path.parent/'binary.sha256').read_text().strip())
    viewports.add(re.search(r'WIDTH=(\d+)\\nHEIGHT=(\d+)',text).groups())
assert len(hashes)==1 and len(viewports)==1, 'comparison binary or viewport differs'
with (args.directory/'summary.csv').open('w',newline='') as file:
    writer=csv.DictWriter(file,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)
for geometry in ['rows','columns']:
    group=[row for row in rows if row['geometry']==geometry]
    print(geometry, {key:round(statistics.median(row[key] for row in group),4) for key in ['fps','down_p50_ms','down_p95_ms','handles_mean_ms','reconnect_mean_ms']})

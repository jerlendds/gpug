#!/usr/bin/env python3
"""Summarize camera replays without requiring an active layout simulation."""
import argparse
import csv
import re
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory', type=Path)
args = parser.parse_args()
rows = []
for path in sorted(args.directory.glob('*.log')):
    text = path.read_text()
    case = path.stem.split('-', 1)[1]
    active = text.split(f'STAGE,{case},start\n')[1].split(f'STAGE,{case},end')[0]
    intervals = sorted(float(v) for v in re.findall(r'^FRAME,([0-9.]+)', active, re.M))
    cameras = [tuple(map(float, row)) for row in re.findall(r'^CAMERA,([^,]+),([^,]+),([^\n]+)', active, re.M)]
    assert intervals and cameras, f'{case}: missing completed paints or camera samples'
    if case in ['zoom-in', 'zoom-out']:
        assert len({v[0] for v in cameras}) > 1, f'{case}: zoom never changed'
        assert (cameras[-1][0] > cameras[0][0]) == (case == 'zoom-in'), f'{case}: wrong zoom direction'
    if case == 'drag':
        assert active.count('SELECTION_CHANGE\n') == 1, 'drag: selection must publish once, not on every movement'
    if case == 'pan':
        assert len({v[1:] for v in cameras}) > 1, 'pan: camera never moved'
    row = dict(case=case, frames=len(intervals), fps=len(intervals)/8,
               p50_ms=intervals[round((len(intervals)-1)*.5)],
               p95_ms=intervals[round((len(intervals)-1)*.95)],
               p99_ms=intervals[round((len(intervals)-1)*.99)],
               max_ms=intervals[-1], zoom_first=cameras[0][0], zoom_last=cameras[-1][0])
    rows.append(row)
    print(row)
if rows:
    with (args.directory/'camera-summary.csv').open('w', newline='') as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)

#!/usr/bin/env python3
"""Summarize the eight-second input windows; optionally verify fixed-path replays."""
import argparse
import csv
from pathlib import Path
import re
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('directory', type=Path)
parser.add_argument('--verify', action='store_true')
parser.add_argument('--case', action='append', help='summarize only selected cases')
args = parser.parse_args()
rows = []
for path in sorted(args.directory.glob('*.log')):
    text = path.read_text()
    case = path.stem.split('-', 1)[1]
    if args.case and case not in args.case:
        continue
    if f'STAGE,{case},end' not in text:
        continue
    active = text.split(f'STAGE,{case},start\n', 1)[1].split(f'STAGE,{case},end', 1)[0]
    intervals = [float(v) for v in re.findall(r'^FRAME,([0-9.]+)', active, re.M)]
    ticks = [int(v) for v in re.findall(r'^FRAME,[0-9.]+,(\d+)', active, re.M)]
    p95 = sorted(intervals)[round((len(intervals)-1)*.95)] if intervals else None
    callbacks = [float(v) for v in re.findall(r'pointer(?:_batch)?_ms p50=[0-9.]+ p95=([0-9.]+)', active)]
    handles = sum(int(v) for v in re.findall(r'\n  handles\s+[0-9.]+ ms/frame  \((\d+) calls\)', active))
    reconnect = sum(int(v) for v in re.findall(r'\n  reconnect\s+[0-9.]+ ms/frame  \((\d+) calls\)', active))
    drag_updates = len(re.findall(r'^DRAG_CHANGE,\d+,true$', active, re.M))
    final_drag = len(re.findall(r'^DRAG_CHANGE,\d+,false$', text, re.M))
    row = dict(case=case, frames=len(intervals), fps=len(intervals)/8, frame_p95_ms=p95,
               max_frame_gap_ms=max(intervals) if intervals else None,
               callback_p95_max_ms=max(callbacks) if callbacks else None,
               layout_steps=ticks[-1]-ticks[0] if ticks else 0,
               handle_calls=handles if intervals else None, reconnect_calls=reconnect if intervals else None,
               drag_updates=drag_updates, drag_releases=final_drag)
    rows.append(row)
    print(row)
    if args.verify:
        assert len(intervals) >= 30, f'{case}: paint stalled during input'
        assert row['layout_steps'] > 0, f'{case}: layout stopped during input'
        if case in ['empty','hover']:
            assert handles == 0 and reconnect == 0, f'{case}: hover still performs picking'
        if case == 'drag':
            assert drag_updates > 0 and final_drag > 0, 'node drag did not update and finish'
            assert drag_updates <= len(intervals)+2, 'node motion was not coalesced to frames'
if rows:
    with (args.directory/('summary-'+'-'.join(args.case)+'.csv' if args.case else 'summary.csv')).open('w',newline='') as file:
        writer=csv.DictWriter(file,fieldnames=list(rows[0])); writer.writeheader(); writer.writerows(rows)

#!/usr/bin/env python3
"""Profile real X11 pointer traffic in an isolated GPUG window (requires XTest)."""
import argparse
import ctypes as C
import math
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--nodes', type=int, default=10000)
parser.add_argument('--case', action='append', choices=['idle','empty','hover','pan','drag','pick','zoom-in','zoom-out'])
parser.add_argument('--legacy-hover', action='store_true', help='requires benchmark-interaction feature')
parser.add_argument('--geometry', choices=['rows','columns'], default='columns')
parser.add_argument('--compare-geometry', action='store_true')
parser.add_argument('--pairs', type=int, default=3)
parser.add_argument('--threads', type=int, default=1, help='Rayon threads; zero uses its default')
parser.add_argument('--all-edges', action='store_true')
parser.add_argument('--camera', choices=['follow', 'free'], default='follow')
parser.add_argument('--static', action='store_true', help='pause layout to isolate camera and drawing costs')
parser.add_argument('--reference-scene', action='store_true', help='same-binary retained scene baseline')
args = parser.parse_args()
if args.compare_geometry:
    if args.pairs < 1: parser.error('pairs must be positive')
    for pair in range(1, args.pairs+1):
        order = ['rows','columns'] if pair % 2 else ['columns','rows']
        for geometry in order:
            subprocess.run([sys.executable, __file__, '--output', str(args.output / f'pair-{pair}-{geometry}'), '--nodes', str(args.nodes), '--case', 'pick', '--geometry', geometry, '--threads', str(args.threads), '--camera', args.camera] + (['--all-edges'] if args.all_edges else []) + (['--static'] if args.static else []), check=True)
    raise SystemExit(0)
args.output.mkdir(parents=True, exist_ok=True)
(args.output / 'binary.sha256').write_text(hashlib.sha256(Path('target/release/examples/interaction_benchmark').read_bytes()).hexdigest()+'\n')
(args.output / 'options.json').write_text(json.dumps(vars(args), default=str, indent=2)+'\n')
x11 = C.CDLL('libX11.so.6')
xtest = C.CDLL('libXtst.so.6')
x11.XOpenDisplay.restype = C.c_void_p
x11.XOpenDisplay.argtypes = [C.c_char_p]
display = x11.XOpenDisplay(None)
if not display: raise RuntimeError('X11 display unavailable')
x11.XFlush.argtypes = [C.c_void_p]
xtest.XTestFakeMotionEvent.argtypes = [C.c_void_p,C.c_int,C.c_int,C.c_int,C.c_ulong]
xtest.XTestFakeButtonEvent.argtypes = [C.c_void_p,C.c_uint,C.c_int,C.c_ulong]
def move(x,y):
    xtest.XTestFakeMotionEvent(display,-1,int(x),int(y),0)
    x11.XFlush(display)
def button(down):
    xtest.XTestFakeButtonEvent(display,1,int(down),0)
    x11.XFlush(display)
for case in args.case or ['idle','empty','hover','pan','drag']:
    path = args.output / f'{args.nodes}-{case}.log'
    unit = f'gpug-input-{os.getpid()}-{case}.scope'
    print(f'RUN {case}',flush=True)
    env = dict(os.environ, GPUG_PICK_GEOMETRY=args.geometry, GPUG_BENCH_BACKEND='vortex', GPUG_LEGACY_HOVER=str(int(args.legacy_hover)), GPUG_PROFILE='1', GPUG_PROFILE_INTERVAL='30', GPUG_NODE_COUNT=str(args.nodes), GPUG_INPUT_CASE=case, RAYON_NUM_THREADS=str(args.threads), GPUG_ALL_EDGES=str(int(args.all_edges)), GPUG_CAMERA=args.camera, GPUG_STATIC=str(int(args.static)), GPUG_REFERENCE_SCENE=str(int(args.reference_scene)))
    if args.threads == 0: env.pop('RAYON_NUM_THREADS', None)
    with path.open('w') as log:
        process = subprocess.Popen(['systemd-run','--user','--scope','--quiet',f'--unit={unit}','-p','MemoryMax=6G','-p','MemorySwapMax=0','--','target/release/examples/interaction_benchmark'],env=env,stdout=log,stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic()+15
            window = None
            while time.monotonic()<deadline:
                found = subprocess.run(['xdotool','search','--class','gpug-interaction-benchmark'],capture_output=True,text=True)
                if found.returncode == 0:
                    window = found.stdout.splitlines()[-1]
                    break
                if process.poll() is not None: raise RuntimeError(path.read_text())
                time.sleep(.1)
            if window is None: raise RuntimeError('benchmark window not found')
            subprocess.run(['xdotool','windowactivate','--sync',window],check=True)
            geometry = subprocess.check_output(['xdotool','getwindowgeometry','--shell',window],text=True)
            geo = dict(line.split('=',1) for line in geometry.splitlines() if '=' in line)
            origin_x,origin_y = int(geo['X']),int(geo['Y'])
            width,height=int(geo['WIDTH']),int(geo['HEIGHT'])
            log.write(f'WINDOW,{window},{geometry!r},pick_geometry={args.geometry}\n'); log.flush()
            move(origin_x-5,origin_y-5)
            time.sleep(3)
            log.write(f'\nSTAGE,{case},start\n'); log.flush()
            start_x,start_y = width*.88,height*.25
            if case == 'drag':
                targets=[line for line in path.read_text().splitlines() if line.startswith('POINTER_TARGET,')]
                _,x,y=targets[-1].split(','); start_x,start_y=float(x),float(y)
            if case != 'idle': move(origin_x+start_x,origin_y+start_y)
            if case in ['pan','drag']: button(True)
            started=time.monotonic(); sent=0; next_click=0.0
            while time.monotonic()-started<8:
                elapsed=time.monotonic()-started
                if case != 'idle':
                    x,y = ((width*.5+70*math.sin(elapsed*3),height*.5) if case=='hover' else (start_x+25*math.sin(elapsed*3),start_y+10*math.sin(elapsed*2)))
                    move(origin_x+x,origin_y+y); sent+=1
                    if case in ['zoom-in','zoom-out'] and elapsed >= next_click:
                        wheel = 4 if case == 'zoom-in' else 5
                        xtest.XTestFakeButtonEvent(display,wheel,1,0)
                        xtest.XTestFakeButtonEvent(display,wheel,0,0)
                        x11.XFlush(display)
                        next_click=elapsed+.25
                    if case == 'pick' and elapsed >= next_click:
                        button(True); button(False)
                        next_click=elapsed+.0667
                time.sleep(.004)
            button(False)
            log.write(f'\nSTAGE,{case},end,sent={sent}\n'); log.flush()
            time.sleep(2)
        finally:
            button(False)
            subprocess.run(['systemctl','--user','stop',unit],capture_output=True)
            process.wait(timeout=10)
    print(f'DONE {case}: {path}',flush=True)

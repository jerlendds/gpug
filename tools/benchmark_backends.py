#!/usr/bin/env python3
"""Run alternating desktop Vortex/Vec replays; never build during measurements."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "benchmarks/backing")
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--samples", type=int, default=180)
    parser.add_argument("--case", action="append", choices=["10000", "100000", "layout"],
                        help="run selected cases; defaults to all three")
    args = parser.parse_args()
    if args.pairs < 1 or args.samples < 60:
        parser.error("pairs must be positive and samples at least 60")
    args.output.mkdir(parents=True, exist_ok=True)
    binary = ROOT / "target/release/examples/backing_benchmark"
    if not binary.is_file():
        parser.error("build the release example with --features benchmark-backends first")
    rows = []
    metadata = {"samples": args.samples, "warmup": 60, "pairs": args.pairs,
                "memory_max": "6G", "memory_swap_max": 0,
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "rayon_threads": 1, "cpu": subprocess.check_output(["lscpu"], text=True),
                "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip()}
    (args.output / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    cases = {"10000": (10_000, False), "100000": (100_000, False), "layout": (10_000, True)}
    for case in args.case or cases:
        count, layout = cases[case]
        for pair in range(args.pairs):
            comparisons = []
            order = ["vortex", "vec"] if pair % 2 == 0 else ["vec", "vortex"]
            for backend in order:
                name = f"grid-{count}-layout-{int(layout)}-pair-{pair + 1}-{backend}"
                path = args.output / f"{name}.log"
                env = dict(os.environ, GPUG_BENCH_BACKEND=backend,
                           GPUG_BENCH_NODES=str(count), GPUG_BENCH_WORKLOAD="grid",
                           GPUG_BENCH_LAYOUT=str(int(layout)), GPUG_BENCH_SAMPLES=str(args.samples),
                           RAYON_NUM_THREADS="1", GPUG_PROFILE="1", GPUG_PROFILE_INTERVAL="60")
                print(f"RUN {name}", flush=True)
                with path.open("w") as log:
                    # Isolate the GUI from the editor's cgroup. A failing driver
                    # or backend must not exhaust memory in the VSCode scope.
                    subprocess.run(["systemd-run", "--user", "--scope", "--quiet",
                                    "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0", "--",
                                    "/usr/bin/time", "-f", "RSS_KIB,%M", str(binary)],
                                   cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   check=True, timeout=600)
                lines = path.read_text().splitlines()
                result = next(line.split(",") for line in lines if line.startswith("RESULT,"))
                if result[1] != backend:
                    raise RuntimeError("binary did not select the requested backing model")
                viewport = next(line for line in lines if line.startswith("VIEWPORT,"))
                final = next(line for line in lines if line.startswith("FINAL,"))
                comparisons.append((viewport, final))
                row = dict(pair=pair + 1, backend=backend, workload=result[2], nodes=int(result[3]),
                           frames=int(result[4]), fps=float(result[5]), p50_ms=float(result[6]),
                           p95_ms=float(result[7]), p99_ms=float(result[8]), layout=layout,
                           source_ms=float(next(line.split(",")[1] for line in lines if line.startswith("SOURCE,"))),
                           install_ms=float(next(line.split(",")[1] for line in lines if line.startswith("INSTALL,"))),
                           peak_rss_kib=int(next(line.split(",")[1] for line in lines if line.startswith("RSS_KIB,"))),
                           viewport=viewport.removeprefix("VIEWPORT,"), final=final.removeprefix("FINAL,"))
                rows.append(row)
                with (args.output / "results.csv").open("w", newline="") as file:
                    writer = csv.DictWriter(file, fieldnames=list(row))
                    writer.writeheader()
                    writer.writerows(rows)
                print(f"DONE {name}: {row['fps']:.3f} FPS; p95 {row['p95_ms']:.3f} ms", flush=True)
            if comparisons[0] != comparisons[1]:
                raise RuntimeError(f"viewport or final graph differs in pair {pair + 1}: {comparisons}")
    print(f"Saved {len(rows)} runs to {args.output}", flush=True)


if __name__ == "__main__":
    main()

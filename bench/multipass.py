#!/usr/bin/env python3
"""Run absbench.py several times and report the median of the *pass* medians.

Cold compare is not reproducible inside a pass: whole-pass medians for the same
range swing 2x between passes (rg91 26/54/68 ms, stress2000 33/50/52 ms). A
single pass is therefore not a publishable number. This wrapper runs the harness
N times, parses each SUMMARY block, and prints the per-range median of those
pass medians plus the observed min/max so the spread is on the record.

Usage:
  python bench/multipass.py --passes 3 --reps 5                 # 16 threads
  python bench/multipass.py --passes 3 --reps 5 --threads 1     # 1 thread
"""
import argparse
import re
import statistics
import subprocess
import sys
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

ROW = re.compile(r"^(rg91|rg252|tokio867|libuv495|stress2000|stressgiant)\s+"
                 r"([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--passes", type=int, default=3)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--warm", type=int, default=200)
    ap.add_argument("--threads", type=int, default=None)
    ap.add_argument("--port", type=int, default=8950)
    args = ap.parse_args()

    # harness appends to the shared raw log; keep this exploratory run out of it
    raw = os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt")
    saved = None
    if os.path.exists(raw):
        with open(raw, "rb") as f:
            saved = f.read()

    per_range = {}
    git = {}
    for p in range(args.passes):
        cmd = [sys.executable, os.path.join(HERE, "absbench.py"),
               "--warm", str(args.warm), "--reps", str(args.reps),
               "--port", str(args.port + p)]
        if args.threads is not None:
            cmd += ["--threads", str(args.threads)]
        print(f"--- pass {p + 1}/{args.passes} ---", flush=True)
        out = subprocess.run(cmd, capture_output=True, text=True).stdout
        for line in out.splitlines():
            m = ROW.match(line.strip())
            if m:
                tag, g, cold, disk, warm = m.groups()
                per_range.setdefault(tag, []).append((float(g), float(cold), float(disk), float(warm)))
                git[tag] = float(g)

    if saved is not None:
        with open(raw, "wb") as f:
            f.write(saved)  # restore: this was exploratory, do not pollute the log

    mode = f"XLR8_THREADS={args.threads}" if args.threads else "16 threads (default)"
    print()
    print(f"### median of {args.passes} passes of absbench --reps {args.reps}  ({mode})")
    print()
    print("| range | git | cold med | cold min..max | disk med | warm med |")
    print("|---|---:|---:|---|---:|---:|")
    for tag, rows in per_range.items():
        colds = [r[1] for r in rows]
        disks = [r[2] for r in rows]
        warms = [r[3] for r in rows]
        print(f"| {tag} | {git[tag]:.1f} ms | {statistics.median(colds):.1f} ms | "
              f"{min(colds):.1f}..{max(colds):.1f} | {statistics.median(disks):.1f} ms | "
              f"{statistics.median(warms):.2f} ms |")


if __name__ == "__main__":
    main()